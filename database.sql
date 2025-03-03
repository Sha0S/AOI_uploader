CREATE TABLE "SMT_AOI_RESULTS" (
	"Serial_NMBR" VARCHAR(30) NOT NULL,
	"Date_Time" DATETIME NOT NULL,
	"Board_NMBR" TINYINT NOT NULL,
	"Program" VARCHAR(30) NOT NULL,
	"Station" VARCHAR(30) NOT NULL,
	"Operator" VARCHAR(30) NULL DEFAULT 'NULL',
	"Result" VARCHAR(10) NOT NULL,
	"Failed" VARCHAR(max) NULL DEFAULT 'NULL',
	"Pseudo_error" VARCHAR(max) NULL DEFAULT 'NULL',
	CONSTRAINT "AOI_Serial_PK" PRIMARY KEY CLUSTERED ("Serial_NMBR", "Date_Time") WITH (IGNORE_DUP_KEY = ON)
)
;