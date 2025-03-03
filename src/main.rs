#![allow(non_snake_case)]

use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use log::{debug, error, info, warn};
use std::{
    fs, path::{Path, PathBuf}, sync::mpsc::{self, SyncSender}, time::Duration
};
use tiberius::{Client, Query};
use tokio::{net::TcpStream, time::sleep};
use tokio_util::compat::TokioAsyncWriteCompatExt;
use tray_item::{IconSource, TrayItem};

mod panel;

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    env_logger::init();
    info!("Starting uploader");

    let (tx, rx) = mpsc::sync_channel(1);
    let sql_tx = tx.clone();

    // SQL uploader thread
    tokio::spawn(async move {
        
        let config = Config::load();
        if config.is_err() {
            error!("Failed to load configuration! Terminating.");
            sql_tx.send(Message::FatalError).unwrap();
            return;
        }
        let config = config.unwrap();
        let log_dir = PathBuf::from(config.AOI_dir.clone());
        let delta_t = Duration::from_secs(config.AOI_deltat);

        let mut client = 
        loop {
            if let Ok(client) =  create_connection(&config).await {
                break client;
            }

            sql_tx.send(Message::SetIcon(IconCollor::Red)).unwrap();
            error!("Failed to connect to the SQL server, retrying in 60s.");
            sleep(Duration::from_secs(60)).await;
        }
        ;        


        sql_tx.send(Message::SetIcon(IconCollor::Green)).unwrap();

        loop {

            // 0 - check connection, reconnect if needed
            loop {
                match client.execute("SELECT 1", &[]).await {
                    Ok(_) => {
                        break;
                    }
                    Err(_) => {
                        warn!("Connection to DB lost, reconnecting!");
                        client = 
                        loop {
                            if let Ok(client) =  create_connection(&config).await {
                                break client;
                            }

                            sql_tx.send(Message::SetIcon(IconCollor::Red)).unwrap();
                            error!("Failed to connect to the SQL server, retrying in 60s.");
                            sleep(Duration::from_secs(60)).await;
                        }
                        ;  
                    }
                }
            }


            debug!("AOI auto update started");
            let start_time = chrono::Local::now();
            

            // 1 - get date_time of the last update
            if let Ok(last_date) = get_last_date() {
                let last_date = last_date - delta_t; 

                // 2 - get possible directories
                let target_dirs = get_subdirs_for_aoi(&log_dir, &last_date);

                // 3 - get logs
                if let Ok(logs) = get_logs(target_dirs, last_date) {
                    // 4 - process_logs

                    let mut processed_logs = Vec::new();
                    for log in logs {
                        if let Ok(plog) = panel::parse_xml(&log, &config.AOI_line) {
                            processed_logs.push(plog);
                        } else {
                            error!("Failed to process log: {:?}", log);
                        }
                    }

                    let mut all_ok = true;
                    // uploading in chunks
                    for chunk in processed_logs.chunks(config.AOI_chunks) {
                        // 5 - craft the SQL query

                        let mut qtext = String::from(
                            "INSERT INTO [dbo].[SMT_AOI_RESULTS] 
                            ([Serial_NMBR], [Board_NMBR], [Program], [Station], [Operator], [Result], [Date_Time], [Failed], [Pseudo_error])
                            VALUES",
                        );

                        for panel in chunk {
                            for board in &panel.Boards {
                                let fails = board.Failed.join(", ");
                                let pseudo = board.Pseudo.join(", ");
                    
                                qtext += &format!(
                                    "('{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}'),",
                                    board.Serial_NMBR,
                                    board.Board_NMBR,
                                    panel.Program,
                                    panel.Station,
                                    panel.Operator,
                                    board.Result,
                                    if panel.Operator.is_empty() {
                                        panel.Inspection_DT
                                    } else {
                                        panel.Repair_DT
                                    },
                                    fails,
                                    pseudo
                                );
                            }
                        }
                        qtext.pop(); // removes last ','

                        // 6 - execute query
                        debug!("Upload: {}", qtext);
                        let query = Query::new(qtext);
                        let result = query.execute(&mut client).await;

                        debug!("Result: {:?}", result);

                        if let Err(e) = result {
                            all_ok = false;
                            error!("Upload failed: {e}");
                        } else {
                            debug!("Upload succesfull!");
                        }
                    }

                    // 7 - update last_date or report the error
                    if all_ok {
                        sql_tx.send(Message::SetIcon(IconCollor::Green)).unwrap();
                        put_last_date(start_time);
                    } else {
                        sql_tx.send(Message::SetIcon(IconCollor::Red)).unwrap();
                        error!("Upload failed - not setting new last_date");
                    }
                } else {
                    error!("Failed to gather logs!");
                }
            } else {
                error!("Failed to read last_date!");
            }

            // wait 5 minutes and repeat
            sleep(Duration::from_secs(300)).await;
        }
    });

    let (mut tray, _) = init_tray(tx.clone());
    let mut last_color = String::new();

    // Tray event loop
    loop {
        match rx.recv() {
            Ok(Message::Quit) => {
                info!("Stoping due user request");
                break;
            }
            Ok(Message::FatalError) => {
                error!("Fatal error encountered, shuting down!");
                break;
            }
            Ok(Message::SetIcon(icon)) => {
                debug!("Icon change requested: {:?}", icon);

                let target_col = match icon {
                    IconCollor::Green => "green-icon",
                    IconCollor::Yellow => "yellow-icon",
                    IconCollor::Red => "red-icon",
                    IconCollor::Grey => "grey-icon",
                    IconCollor::Purple => "purple-icon",
                };

                if target_col == last_color {
                    continue;
                }
                if tray.set_icon(IconSource::Resource(target_col)).is_ok() {
                    debug!("Icon set to: {target_col}");
                    last_color = target_col.to_owned();
                } else {
                    warn!("Failed to change icon to: {target_col}");
                }
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(Default)]
struct Config {
    server: String,
    database: String,
    password: String,
    username: String,

    AOI_dir: String,
    AOI_line: String,
    AOI_chunks: usize,
    AOI_deltat: u64,
}

impl Config {
    fn load() -> anyhow::Result<Config> {
        let mut c = Config::default();

        if let Ok(config) = ini::Ini::load_from_file("config.ini") {
            if let Some(jvserver) = config.section(Some("JVSERVER")) {
                // mandatory fields:
                if let Some(server) = jvserver.get("SERVER") {
                    c.server = server.to_owned();
                }
                if let Some(password) = jvserver.get("PASSWORD") {
                    c.password = password.to_owned();
                }
                if let Some(username) = jvserver.get("USERNAME") {
                    c.username = username.to_owned();
                }
                if let Some(database) = jvserver.get("DATABASE") {
                    c.database = database.to_owned();
                }

                if c.server.is_empty()
                    || c.password.is_empty()
                    || c.username.is_empty()
                    || c.database.is_empty()
                {
                    return Err(anyhow::Error::msg(
                        "ER: Missing [JVSERVER] fields from configuration file!",
                    ));
                }
            } else {
                return Err(anyhow::Error::msg("ER: Could not find [JVSERVER] field!"));
            }

            if let Some(app) = config.section(Some("AOI")) {
                if let Some(dir) = app.get("DIR") {
                    c.AOI_dir = dir.to_owned();
                }

                if let Some(dir) = app.get("LINE") {
                    c.AOI_line = dir.to_owned();
                }

                if let Some(chunks) = app.get("CHUNKS") {
                    c.AOI_chunks = chunks.parse().unwrap_or(10);
                }

                if let Some(chunks) = app.get("DELTA_T") {
                    c.AOI_deltat = chunks.parse().unwrap_or(0);
                }

                if c.AOI_dir.is_empty()
                    || c.AOI_line.is_empty()
                {
                    return Err(anyhow::Error::msg(
                        "ER: Missing [AOI] fields from configuration file!",
                    ));
                }
            } else {
                return Err(anyhow::Error::msg("ER: Could not find [AOI] field!"));
            }

        } else {
            return Err(anyhow::Error::msg(
                "ER: Could not read configuration file! [.\\config.ini]",
            ));
        }

        Ok(c)
    }
}

async fn connect(
    tib_config: tiberius::Config,
) -> anyhow::Result<tiberius::Client<tokio_util::compat::Compat<TcpStream>>> {
    let tcp = TcpStream::connect(tib_config.get_addr()).await?;
    tcp.set_nodelay(true)?;
    let client = Client::connect(tib_config, tcp.compat_write()).await?;

    Ok(client)
}

async fn create_connection(config: &Config) -> Result<Client<tokio_util::compat::Compat<TcpStream>>> {
        // Tiberius configuartion:

        let sql_server = config.server.to_owned();
        let sql_user = config.username.to_owned();
        let sql_pass = config.password.to_owned();
    
        let mut tib_config = tiberius::Config::new();
        tib_config.host(sql_server);
        tib_config.authentication(tiberius::AuthMethod::sql_server(sql_user, sql_pass));
        tib_config.trust_cert(); // Most likely not needed.
    
        let mut client_tmp = connect(tib_config.clone()).await;
        let mut tries = 0;
        while client_tmp.is_err() && tries < 3 {
            client_tmp = connect(tib_config.clone()).await;
            tries += 1;
        }
    
        if client_tmp.is_err() {
            bail!("Connection to DB failed!")
        }
        let mut client = client_tmp?;
    
        // USE [DB]
        let qtext = format!("USE [{}]", config.database);
        debug!("USE DB: {}", qtext);
        let query = Query::new(qtext);
        query.execute(&mut client).await?;

        Ok(client)
}

fn get_logs(target_dirs: Vec<PathBuf>, last_date: DateTime<Local>) -> Result<Vec<PathBuf>> {
    let mut ret = Vec::new();

    for dir in target_dirs {
        for file in fs::read_dir(dir)? {
            let file = file?;
            let path = file.path();

            if path.is_file() && path.extension().is_some_and(|f| f == "xml" || f =="XML") {
                if let Ok(x) = path.metadata() {
                    let ct: chrono::DateTime<chrono::Local> = x.modified().unwrap().into();
                    if ct >= last_date {

                        // filtering temporary files from AOI / AXI
                        if let Some(filestem) = path.file_stem() {
                            let filestem = filestem.to_string_lossy();
                            if !(filestem.ends_with("_AOI") || filestem.ends_with("_AXI")) {
                                ret.push(path);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(ret)
}

fn get_last_date() -> Result<DateTime<Local>> {
    let last_date = fs::read_to_string("last_date.txt");

    if last_date.is_err() {
        error!("Error reading last_date.txt!");
        bail!("Error reading last_date.txt!");
    }

    let last_date = last_date.unwrap();
    debug!("Last date: {}", last_date);

    let last_date = chrono::NaiveDateTime::parse_from_str(&last_date, "%Y-%m-%d %H:%M:%S");

    if last_date.is_err() {
        error!("Error converting last_date!");
        bail!("Error converting last_date!");
    }

    let last_date = last_date.unwrap().and_local_timezone(chrono::Local);
    let last_date = match last_date {
        chrono::offset::LocalResult::Single(t) => t,
        chrono::offset::LocalResult::Ambiguous(earliest, _) => earliest,
        chrono::offset::LocalResult::None => {
            error!("Error converting last_date! LocalResult::None!");
            bail!("Error converting last_date! LocalResult::None!");
        }
    };

    Ok(last_date)
}

fn put_last_date(end_time: DateTime<Local>) {
    let output_string = end_time.format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = fs::write("last_date.txt", output_string);
}

fn get_subdirs_for_aoi(log_dir: &Path, start: &chrono::DateTime<chrono::Local>) -> Vec<PathBuf> {
    let mut ret = Vec::new();

    let mut start_date = start.date_naive();
    let end_date = chrono::Local::now().date_naive();

    while start_date <= end_date {
        debug!("\tdate: {}", start_date);

        let sub_dir = start_date.format("%Y_%m_%d");

        debug!("\tsubdir: {}", sub_dir);

        let new_path = log_dir.join(sub_dir.to_string());
        debug!("\tfull path: {:?}", new_path);

        if new_path.exists() {
            debug!("\t\tsubdir exists");
            ret.push(new_path);
        }

        start_date = start_date.succ_opt().unwrap();
    }

    ret
}

#[derive(Debug)]
pub enum IconCollor {
    Green,
    Yellow,
    Red,
    Grey,
    Purple,
}
pub enum Message {
    Quit,
    FatalError,
    SetIcon(IconCollor),
}

pub fn init_tray(tx: SyncSender<Message>) -> (TrayItem, Vec<u32>) {
    let mut ret = Vec::new();

    let mut tray = TrayItem::new("AOI Uploader", IconSource::Resource("red-icon")).unwrap();

    ret.push(
        // 0
        tray.inner_mut().add_label_with_id("AOI Uploader").unwrap(),
    );

    tray.inner_mut().add_separator().unwrap();

    let quit_tx = tx.clone();
    tray.add_menu_item("Quit", move || {
        quit_tx.send(Message::Quit).unwrap();
    })
    .unwrap();

    (tray, ret)
}
