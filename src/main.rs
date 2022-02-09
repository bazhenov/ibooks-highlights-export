use std::{
    ffi::OsStr,
    fs::{self, File},
    path::PathBuf,
};

use anyhow::Result;
use rusqlite::Connection;
use thiserror::Error;

const LIBRARY_DB_PATH: &str = "~/Library/Containers/com.apple.iBooksX/Data/Documents/BKLibrary";

#[derive(Error, Debug)]
enum Errors {
    #[error("No home dir can be detected")]
    NoHomeDir,

    #[error("iBooks annotation database not found. Are you sure iBooks is installed?")]
    NoAnnotationDbFound,
}

struct Annotation {
    selected_text: Option<String>,
    note: Option<String>,
}

fn main() -> Result<()> {
    let db_path = locate_annotation_database()?.ok_or(Errors::NoAnnotationDbFound)?;
    println!("{:?}", db_path);
    let connection = Connection::open(db_path)?;

    let mut stms =
        connection.prepare("select ZANNOTATIONSELECTEDTEXT, ZANNOTATIONNOTE from ZAEANNOTATION")?;
    let annotations = stms.query_map([], |row| {
        Ok(Annotation {
            selected_text: row.get(0)?,
            note: row.get(1)?,
        })
    })?;

    for a in annotations {
        let a = a?;
        println!(
            "Text: {}",
            a.selected_text.as_ref().map(String::as_str).unwrap_or("-")
        );
        if let Some(note) = a.note {
            println!("Note: {}", note);
        }
        println!();
    }
    Ok(())
}

fn locate_annotation_database() -> Result<Option<PathBuf>> {
    const ANNOTATION_DB_PATH: &str =
        "Library/Containers/com.apple.iBooksX/Data/Documents/AEAnnotation";

    let mut dir = dirs::home_dir().ok_or(Errors::NoHomeDir)?;
    dir.push(ANNOTATION_DB_PATH);

    for file in fs::read_dir(dir)? {
        let path = file?.path();
        let extension = path.extension().map(OsStr::to_str).flatten().unwrap_or("");
        if extension == "sqlite" {
            return Ok(Some(path));
        }
    }

    Ok(None)
}
