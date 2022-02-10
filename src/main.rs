use std::{
    collections::HashMap,
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

    let connection = Connection::open(annotation_db)?;
    connection.execute("ATTACH DATABASE ? AS l", [library_db.to_str()])?;

    let last_sync_date = last_sync_time::read()?;
    log::debug!("Last sync date: {:?}", last_sync_date);

    let last_sync_date = last_sync_date.map(|s| s.timestamp()).unwrap_or(0);

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
        last_sync_time::update(time)?;
    }

    Ok(())
}

mod last_sync_time {
    use super::*;
    const FILE_NAME: &str = "./.last_sync";

    pub fn read() -> Result<Option<DateTime<Utc>>> {
        if !Path::new(FILE_NAME).exists() {
            return Ok(None);
        }

        let data = fs::read(FILE_NAME)?;

        let string = String::from_utf8(data)?;
        let date = DateTime::parse_from_rfc3339(&string)?.with_timezone(&Utc);
        Ok(Some(date))
    }

    pub fn update(ts: DateTime<Utc>) -> Result<()> {
        fs::write(FILE_NAME, ts.to_rfc3339())?;
        Ok(())
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
