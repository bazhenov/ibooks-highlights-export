use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use clap::Parser;
use log::{debug, error};
use rusqlite::Connection;
use serde::Serialize;
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Parser, Debug)]
#[clap(author, version = "0.1", about, long_about = None)]
struct Args {
    /// Update sync date at the end
    #[clap(long)]
    update: bool,

    /// Output annotation in JSON format
    #[clap(long, short)]
    json: bool,

    /// Output annotation in table format
    #[clap(long, short)]
    table: bool,

    /// Read all annotations, not from last sync time
    #[clap(short)]
    all: bool,
}

#[derive(Error, Debug)]
enum Errors {
    #[error("No home dir can be detected")]
    NoHomeDir,

    #[error("iBooks database not found. Are you sure iBooks is installed?")]
    NoDbFound,

    #[error("Processing annotation")]
    ContextProcessingAnnotation,

    #[error("Unable to find program location")]
    UnableToFindProgramLocation,

    #[error("Unable to write sync-file")]
    UnableToWriteSyncFile,

    #[error("Unable to read sync-file")]
    UnableToReadSyncFile,
}

#[derive(Serialize)]
struct Annotation {
    selected_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    anotation_time: DateTime<Utc>,
    book_title: String,
}

fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    let (annotation_db, library_db) = locate_annotation_database()?
        .zip(locate_library_database()?)
        .ok_or(Errors::NoDbFound)?;

    debug!("Library database location: {:?}", &library_db);
    debug!("Annotation database location: {:?}", &annotation_db);

    let last_sync_file = LastSyncFile::find()?;
    debug!("Last sync file: {:?}", last_sync_file.0);

    let last_sync = if args.all {
        None
    } else {
        last_sync_file.read()?
    };
    debug!("Last sync date: {:?}", last_sync);

    let annotations = read_annotations(annotation_db, library_db, last_sync)?;
    let new_last_sync_time = annotations.iter().map(|a| a.anotation_time).max();

    if args.json {
        println!("{}", format::Json(annotations));
    } else if args.table {
        println!("{}", format::Table(annotations));
    } else {
        println!("{}", format::Logseq(annotations));
    }

    if args.update {
        if let Some(time) = new_last_sync_time {
            debug!("Updating last sync time: {}", time);
            last_sync_file.update(time)?;
        }
    }

    Ok(())
}

fn read_annotations(
    annotation_db: impl AsRef<Path>,
    library_db: impl AsRef<Path>,
    created_after: Option<DateTime<Utc>>,
) -> Result<Vec<Annotation>> {
    let connection = Connection::open(annotation_db)?;
    connection.execute("ATTACH DATABASE ? AS l", [library_db.as_ref().to_str()])?;

    // Here I'm using ZFUTUREPROOFING6 instead of ZANNOTATIONMODIFICATIONDATE beacuse
    // latter works unreliably. It seems like ZFUTUREPROOFING6 is annotation created time.
    // At least I've found one other project using it:
    // https://github.com/jay1803/ibook-server/blob/58838a3a1004aeaaa7cd7ebf4ef95edf8cc45ed3/controllers/bookController.js#L124
    let mut stms = connection.prepare(
        "select
            a.ZANNOTATIONSELECTEDTEXT,
            a.ZANNOTATIONNOTE,
            round(a.ZFUTUREPROOFING6),
            l.ZTITLE
         from ZAEANNOTATION a
         inner join ZBKLIBRARYASSET l ON l.ZASSETID = a.ZANNOTATIONASSETID
         where a.ZANNOTATIONSELECTEDTEXT IS NOT NULL AND (a.ZANNOTATIONNOTE != '' OR a.ZANNOTATIONNOTE IS NULL) AND
         round(a.ZFUTUREPROOFING6) > ?
         ORDER BY a.ZFUTUREPROOFING6",
    )?;
    let created_after = created_after.map(|t| t.timestamp()).unwrap_or(0);
    let annotations = stms.query_map([timestamp_to_core_data(created_after)], |row| {
        let ts: f64 = row.get(2)?;
        Ok(Annotation {
            selected_text: row.get(0)?,
            note: row.get(1)?,
            anotation_time: core_data_to_timestamp(ts as i64),
            book_title: row.get(3)?,
        })
    })?;

    annotations
        .map(|r| r.context(Errors::ContextProcessingAnnotation))
        .collect::<Result<Vec<_>>>()
}

struct LastSyncFile(PathBuf);

impl LastSyncFile {
    fn find() -> Result<Self> {
        let mut state_dir = dirs::data_dir().ok_or(Errors::UnableToFindProgramLocation)?;

        state_dir.push("ibooks-export");
        if !state_dir.exists() {
            fs::create_dir_all(&state_dir)?;
        }

        state_dir.push("last_sync");

        Ok(Self(state_dir))
    }

    pub fn read(&self) -> Result<Option<DateTime<Utc>>> {
        if !Path::new(&self.0).exists() {
            return Ok(None);
        }

        let data = fs::read(&self.0).context(Errors::UnableToReadSyncFile)?;

        let string = String::from_utf8(data)?;
        let date = DateTime::parse_from_rfc3339(&string)?.with_timezone(&Utc);
        Ok(Some(date))
    }

    pub fn update(&self, ts: DateTime<Utc>) -> Result<()> {
        fs::write(&self.0, ts.to_rfc3339()).context(Errors::UnableToWriteSyncFile)
    }
}

fn locate_annotation_database() -> Result<Option<PathBuf>> {
    locate_database("Library/Containers/com.apple.iBooksX/Data/Documents/AEAnnotation")
}

fn locate_library_database() -> Result<Option<PathBuf>> {
    locate_database("Library/Containers/com.apple.iBooksX/Data/Documents/BKLibrary")
}

fn locate_database(path: impl AsRef<Path>) -> Result<Option<PathBuf>> {
    let mut dir = dirs::home_dir().ok_or(Errors::NoHomeDir)?;
    dir.push(path);

    for file in fs::read_dir(dir)? {
        let path = file?.path();
        let extension = path.extension().and_then(OsStr::to_str).unwrap_or("");
        if extension == "sqlite" {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

fn core_data_to_timestamp(ts: i64) -> DateTime<Utc> {
    Utc.timestamp(ts + 978307200, 0)
}

fn timestamp_to_core_data(ts: i64) -> i64 {
    ts - 978307200
}

mod format {
    use super::*;
    use std::fmt;
    use term_table::{row::Row, table_cell::TableCell, TableStyle};

    /// Json format for annotations
    pub(crate) struct Json(pub Vec<Annotation>);

    impl fmt::Display for Json {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match serde_json::to_string(&self.0) {
                Ok(json) => {
                    write!(f, "{}", json)?;
                    Ok(())
                }
                Err(e) => {
                    error!("Unable to format error: {}", e);
                    Err(fmt::Error)
                }
            }
        }
    }

    /// Logseq format
    ///
    /// Formatting annotations in logseq format like
    /// ```markdown
    /// - [[Book 1]]
    ///     - > annotation 1
    ///     - > annotation 2
    /// - [[Book 2]]
    ///     - > annotation 1
    /// ```
    pub(crate) struct Logseq(pub Vec<Annotation>);

    impl fmt::Display for Logseq {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let mut annotations_by_book = HashMap::new();
            let annotations = &self.0;
            for a in annotations {
                annotations_by_book
                    .entry(a.book_title.clone())
                    .or_insert_with(Vec::new)
                    .push(a);
            }

            for (book, annotations) in annotations_by_book {
                writeln!(f, "- [[{}]]", book)?;
                for a in annotations {
                    let text = a.selected_text.as_deref().unwrap_or("-");
                    if let Some(note) = &a.note {
                        writeln!(f, "\t\t- {}", note)?;
                        writeln!(f, "\t\t\t- > {}", text)?;
                    } else {
                        writeln!(f, "\t\t- > {}", text)?;
                    }
                }
            }
            Ok(())
        }
    }

    pub(crate) struct Table(pub Vec<Annotation>);

    impl fmt::Display for Table {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let mut table = term_table::Table::new();

            table.max_column_width(120);
            table.style = TableStyle::rounded();

            for annotation in &self.0 {
                if let Some(text) = &annotation.selected_text {
                    let row = Row::new(vec![
                        TableCell::new(&annotation.book_title),
                        TableCell::new(annotation.anotation_time),
                        TableCell::new(text),
                    ]);
                    table.add_row(row);
                }
            }
            write!(f, "{}", table.render())?;
            Ok(())
        }
    }
}
