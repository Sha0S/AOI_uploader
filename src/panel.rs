/*
SQL fields:
- Serial_NMBR: VARCHAR[30]
- Board_NMBR: TINYINT // The board number in the XML is wrong!
- Program: VARCHAR[30]
- Station: VARCHAR[30]
- Operator: VARCHAR[30] (allow NULL)
- Result: VARCHAR[10]
- Date_Time: DATETIME
- Failed: VARCHAR[500] (allow NULL)
*/

use std::path::PathBuf;
use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDateTime};
use log::{debug, error, info};

#[derive(Debug, Default)]
pub struct Panel {
     pub Program: String,
    pub Station: String,
    pub Operator: String,
    pub Repair_DT: NaiveDateTime,
    pub Inspection_DT: NaiveDateTime,

    pub Boards: Vec<Board>,
}

#[derive(Debug, Default, Clone)]
pub struct Board {
    pub Serial_NMBR: String,
    pub Board_NMBR: usize,
    pub Result: String,
    pub Failed: Vec<String>,
    pub Pseudo: Vec<String>,
}

pub fn parse_xml(path: &PathBuf, line: &str) -> Result<Panel> {
    info!("Processing XML: {path:?}");

    let mut ret = Panel::default();

    let file = std::fs::read_to_string(path)?;
    let xml = roxmltree::Document::parse(&file)?;

    let root = xml.root_element();
    let mut repaired = false;
    let mut failed = false;

    if let Some(ginfo) = root
        .children()
        .find(|f| f.has_tag_name("GlobalInformation"))
    {
        for sub_child in ginfo.children().filter(|f| f.is_element()) {
            match sub_child.tag_name().name() {
                /*"Station" => {
                    if let Some(x) = sub_child.children().find(|f| f.has_tag_name("Name")) {
                        ret.Station = x.text().unwrap_or_default().to_owned();
                        debug!("Station: {}", ret.Station);
                    }
                }*/
                "Program" => {
                    if let Some(x) = sub_child
                        .children()
                        .find(|f| f.has_tag_name("InspectionPlanName"))
                    {
                        ret.Program = x.text().unwrap_or_default().to_owned();
                        debug!("Program: {}", ret.Program);
                    }
                }
                "Inspection" => {
                    let date =
                        if let Some(x) = sub_child.children().find(|f| f.has_tag_name("Date")) {
                            if let Some(y) = x.children().find(|f| f.has_tag_name("End")) {
                                y.text().unwrap_or_default()
                            } else {
                                ""
                            }
                        } else {
                            ""
                        };
                    let time =
                        if let Some(x) = sub_child.children().find(|f| f.has_tag_name("Time")) {
                            if let Some(y) = x.children().find(|f| f.has_tag_name("End")) {
                                y.text().unwrap_or_default()
                            } else {
                                ""
                            }
                        } else {
                            ""
                        };

                    if !date.is_empty() && !time.is_empty() {
                        let t = format!("{date} {time}");
                        debug!("Raw time string: {t}");
                        ret.Inspection_DT =
                            NaiveDateTime::parse_from_str(&t, "%Y%m%d %H%M%S").unwrap_or_default();
                        debug!("Date_Time: {:?}", ret.Inspection_DT);
                    }
                }
                "Repair" => {
                    repaired = true;

                    if let Some(x) = sub_child
                        .children()
                        .find(|f| f.has_tag_name("OperatorName"))
                    {
                        ret.Operator = x.text().unwrap_or_default().to_uppercase();
                        debug!("OperatorName: {}", ret.Operator);
                    }

                    let date =
                        if let Some(x) = sub_child.children().find(|f| f.has_tag_name("Date")) {
                            if let Some(y) = x.children().find(|f| f.has_tag_name("End")) {
                                y.text().unwrap_or_default()
                            } else {
                                ""
                            }
                        } else {
                            ""
                        };
                    let time =
                        if let Some(x) = sub_child.children().find(|f| f.has_tag_name("Time")) {
                            if let Some(y) = x.children().find(|f| f.has_tag_name("End")) {
                                y.text().unwrap_or_default()
                            } else {
                                ""
                            }
                        } else {
                            ""
                        };

                    if !date.is_empty() && !time.is_empty() {
                        let t = format!("{date} {time}");
                        debug!("Raw time string: {t}");
                        ret.Repair_DT =
                            NaiveDateTime::parse_from_str(&t, "%Y%m%d %H%M%S").unwrap_or_default();
                        debug!("Date_Time: {:?}", ret.Repair_DT);
                    }
                }
                _ => (),
            }
        }
    } else {
        error!("Could not find <GlobalInformation>!");
        bail!("Could not find <GlobalInformation>!");
    }

    if ret.Program.is_empty()
        || ret.Inspection_DT.year() < 2000
        || (repaired && ret.Repair_DT.year() < 2000)
    {
        error!("Missing mandatory <GlobalInformation> elements!");
        bail!("Missing mandatory <GlobalInformation> elements!");
    }

    if let Some(pcb_info) = root.children().find(|f| f.has_tag_name("PCBInformation")) {
        let count = pcb_info.children().filter(|f| f.is_element()).count();
        debug!("PCB count: {}", count);
        ret.Boards = vec![Board::default(); count];

        for (i, child) in pcb_info
            .children()
            .filter(|f| f.tag_name().name() == "SinglePCB")
            .enumerate()
        {
            let mut serial = String::new();
            let mut result = String::new();

            for sub_child in child.children().filter(|f| f.is_element()) {
                match sub_child.tag_name().name() {
                    "Barcode" => {
                        serial = sub_child.text().unwrap_or_default().to_owned();
                    }
                    "Result" => {
                        result = sub_child.text().unwrap_or_default().to_owned();
                    }
                    _ => {}
                }
            }

            debug!("{i}: serial: {serial}, result: {result}");
            if !serial.is_empty() && !result.is_empty() {
                if result != "PASS" {
                    failed = true;
                }
                ret.Boards[i].Serial_NMBR = serial;
                ret.Boards[i].Result = result;
            } else {
                error!("SinglePCB sub-fields missing!");
                bail!("SinglePCB sub-fields missing!");
            }
        }
    }

    for board in &ret.Boards {
        if board.Serial_NMBR.is_empty() || board.Result.is_empty() {
            error!("Board serial or result is missing!");
            bail!("Board serial or result is missing!");
        }
    }

    if repaired {
        debug!("XML is for repair station. Searching for repair information");
        if let Some(comp_info) = root
            .children()
            .find(|f| f.has_tag_name("ComponentInformation"))
        {
            for window in comp_info.children().filter(|f| f.is_element()) {
                let mut WinID = String::new();
                let mut PCBNumber = String::new();
                let mut Result = String::new();

                for sub_child in window.children().filter(|f| f.is_element()) {
                    match sub_child.tag_name().name() {
                        "WinID" => {
                            WinID = sub_child.text().unwrap_or_default().to_string();
                        }
                        "PCBNumber" => {
                            PCBNumber = sub_child.text().unwrap_or_default().to_string();
                        }
                        "Result" => {
                            if let Some(t) = sub_child
                                .children()
                                .find(|f| f.has_tag_name("ErrorDescription"))
                            {
                                Result = t.text().unwrap_or_default().to_string();
                            }
                        }
                        _ => {}
                    }
                }

                if !(WinID.is_empty() || PCBNumber.is_empty() || Result.is_empty()) {
                    debug!(
                        "Window found! WinID: {WinID}, PCBNumber: {PCBNumber}, Result: {Result}"
                    );

                    
                    if let Ok(x) = PCBNumber.parse::<usize>() {
                        if let Some(board) = ret.Boards.get_mut(x) {
                            if let Some(c) = WinID.rfind('-') {
                                let split = WinID.split_at(c);
                                WinID = split.0.to_string();
                            }

                            if Result != "Pszeudohiba" {
                                if !board.Failed.contains(&WinID) {
                                    board.Failed.push(WinID);
                                }
                            } else if !board.Pseudo.contains(&WinID) {
                                board.Pseudo.push(WinID);
                            }
                        }
                    } else {
                        error!("Could not parse PCBNumber: {PCBNumber}");
                        bail!("Could not parse PCBNumber: {PCBNumber}");
                    }
                    
                } else {
                    error!("Window interpreting error! WinID: {WinID}, PCBNumber: {PCBNumber}, Result: {Result}");
                    bail!("Window interpreting error! WinID: {WinID}, PCBNumber: {PCBNumber}, Result: {Result}");
                }
            }
        }
    } else if failed {
        debug!("XML is for AOI/AXI station. Searching for failed windows.");
        if let Some(comp_info) = root
            .children()
            .find(|f| f.has_tag_name("ComponentInformation"))
        {
            for window in comp_info.children().filter(|f| f.is_element()) {
                let mut WinID = String::new();
                let mut PCBNumber = String::new();
                let mut Result = String::new();

                for sub_child in window.children().filter(|f| f.is_element()) {
                    match sub_child.tag_name().name() {
                        "WinID" => {
                            WinID = sub_child.text().unwrap_or_default().to_string();
                        }
                        "PCBNumber" => {
                            PCBNumber = sub_child.text().unwrap_or_default().to_string();
                        }
                        "Analysis" => {
                            if let Some(t) = sub_child.children().find(|f| f.has_tag_name("Result"))
                            {
                                Result = t.text().unwrap_or_default().to_string();
                            }
                        }
                        _ => {}
                    }
                }

                if !(WinID.is_empty() || PCBNumber.is_empty() || Result.is_empty()) {
                    if Result != "0" {
                        debug!(
                            "Window found! WinID: {WinID}, PCBNumber: {PCBNumber}, Result: {Result}"
                        );

                        if let Ok(x) = PCBNumber.parse::<usize>() {
                            if x == 0 {
                                error!("BoardNumber is 0. Was excepting 1+");
                                bail!("BoardNumber is 0. Was excepting 1+");
                            } else if let Some(board) = ret.Boards.get_mut(x - 1) {
                                if let Some(c) = WinID.rfind('-') {
                                    let split = WinID.split_at(c);
                                    WinID = split.0.to_string();
                                }

                                if !board.Failed.contains(&WinID) {
                                    board.Failed.push(WinID);
                                }
                            } else {
                                error!("Could not find board number {x}");
                                bail!("Could not find board number {x}");
                            }
                        } else {
                            error!("Could not parse PCBNumber: {PCBNumber}");
                            bail!("Could not parse PCBNumber: {PCBNumber}");
                        }
                    }
                } else {
                    error!("Window interpreting error! WinID: {WinID}, PCBNumber: {PCBNumber}, Result: {Result}");
                    bail!("Window interpreting error! WinID: {WinID}, PCBNumber: {PCBNumber}, Result: {Result}");
                }
            }
        }
    }

    // Sort boards, so they will be in the "correct" order
    ret.Boards
        .sort_by(|p1, p2| p1.Serial_NMBR.cmp(&p2.Serial_NMBR));
    // Set board number to the "correct" value
    for (i, b) in ret.Boards.iter_mut().enumerate() {
        b.Board_NMBR = i + 1;
    }

    // Set station name
    ret.Station = if repaired {
        format!("{}_HARAN", line)
    } else {
        format!("{}_AOI_AXI", line)
    };

    info!("Processing OK.");

    Ok(ret)
}