use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use thiserror::Error;

#[derive(Error, Debug)]
enum Errors {
    #[error("No home dir can be detected")]
    NoHomeDir,

    #[error("iBooks database not found. Are you sure iBooks is installed?")]
    NoDbFound,
}

struct Annotation {
    selected_text: Option<String>,
    note: Option<String>,
    creation_date: DateTime<Utc>,
    book_title: String,
}

fn main() -> Result<()> {
    let (annotation_db, library_db) = locate_annotation_database()?
        .zip(locate_library_database()?)
        .ok_or(Errors::NoDbFound)?;

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
         where a.ZANNOTATIONSELECTEDTEXT IS NOT NULL AND (a.ZANNOTATIONNOTE != '' OR a.ZANNOTATIONNOTE IS NULL)
         ORDER BY a.ZANNOTATIONMODIFICATIONDATE",
    )?;
    let annotations = stms.query_map([], |row| {
        let ts: f32 = row.get(2)?;
        Ok(Annotation {
            selected_text: row.get(0)?,
            note: row.get(1)?,
            creation_date: core_data_to_timestamp(ts as i64),
            book_title: row.get(3)?,
        })
    })?;

    let mut annotations_by_book = HashMap::new();
    for a in annotations {
        let a = a?;

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

    Ok(())
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
