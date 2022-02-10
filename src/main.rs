use std::{
    collections::HashMap,
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::Connection;
use thiserror::Error;

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

struct Annotation {
    selected_text: Option<String>,
    note: Option<String>,
    anotation_time: DateTime<Utc>,
    book_title: String,
}

fn main() -> Result<()> {
    env_logger::init();
    let (annotation_db, library_db) = locate_annotation_database()?
        .zip(locate_library_database()?)
        .ok_or(Errors::NoDbFound)?;

    log::debug!("Library database location: {:?}", &library_db);
    log::debug!("Annotation database location: {:?}", &annotation_db);

    let last_sync_file = LastSyncFile::find()?;
    log::debug!("Last sync file: {:?}", last_sync_file.0);
    let last_sync = last_sync_file.read()?;
    log::debug!("Last sync date: {:?}", last_sync);

    let last_sync_date = last_sync.map(|s| s.timestamp()).unwrap_or(0);

    let connection = Connection::open(annotation_db)?;
    connection.execute("ATTACH DATABASE ? AS l", [library_db.to_str()])?;

    let mut stms = connection.prepare(
        "select
            a.ZANNOTATIONSELECTEDTEXT,
            a.ZANNOTATIONNOTE,
            a.ZANNOTATIONMODIFICATIONDATE,
            l.ZTITLE
         from ZAEANNOTATION a
         inner join ZBKLIBRARYASSET l ON l.ZASSETID = a.ZANNOTATIONASSETID
         where a.ZANNOTATIONSELECTEDTEXT IS NOT NULL AND (a.ZANNOTATIONNOTE != '' OR a.ZANNOTATIONNOTE IS NULL) AND
         a.ZANNOTATIONMODIFICATIONDATE > ?
         ORDER BY a.ZANNOTATIONMODIFICATIONDATE",
    )?;
    let annotations = stms.query_map([timestamp_to_core_data(last_sync_date)], |row| {
        let ts: f32 = row.get(2)?;
        Ok(Annotation {
            selected_text: row.get(0)?,
            note: row.get(1)?,
            anotation_time: core_data_to_timestamp(ts as i64),
            book_title: row.get(3)?,
        })
    })?;

    let annotations = annotations
        .map(|r| r.context(Errors::ContextProcessingAnnotation))
        .collect::<Result<Vec<_>>>()?;

    let new_last_sync_time = annotations.iter().map(|a| a.anotation_time).max();

    let mut annotations_by_book = HashMap::new();
    for a in annotations {
        annotations_by_book
            .entry(a.book_title.clone())
            .or_insert_with(Vec::new)
            .push(a);
    }

    for (book, annotations) in annotations_by_book {
        println!();
        println!("- [[{}]]", book);
        for a in annotations {
            let text = a.selected_text.as_ref().map(String::as_str).unwrap_or("-");
            if let Some(note) = a.note {
                println!("\t\t- {}", note);
                println!("\t\t\t- > {}", text);
            } else {
                println!("\t\t- > {}", text);
            }
        }
    }

    if let Some(time) = new_last_sync_time {
        log::debug!("Updating last sync time: {}", time);
        last_sync_file.update(time)?;
    }

    Ok(())
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
        let extension = path.extension().map(OsStr::to_str).flatten().unwrap_or("");
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
