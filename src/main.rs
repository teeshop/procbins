mod string_logger;

use std::{collections::HashSet, ops::Deref};
use sysinfo::{ProcessExt, System, SystemExt};
use zip::write::FileOptions;
use zip::result::{ZipError,ZipResult};
use std::fs::File;
use std::path::PathBuf;
use path_slash::PathBufExt;
use std::io::prelude::*;
use argparse::{ArgumentParser, Store};
use log::{info, warn, error};
use flexi_logger::{Logger, LogTarget, Duplicate, detailed_format};
use string_logger::*;
use std::sync::{Arc, Mutex};
use sha1::{Sha1, Digest};
use string_builder::Builder;

#[cfg(windows)] use std::borrow::Cow;
#[cfg(windows)] use regex::Regex;

struct BinaryStatus {
    files: HashSet<PathBuf>
}

fn main() {
    let log_buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let string_logger = StringLogger::new(log_buffer.clone(), detailed_format);
    Logger::with_str("info")
        .duplicate_to_stderr(Duplicate::Info)
        .log_target(LogTarget::Writer(Box::new(string_logger)))
        .start()
        .unwrap_or_else(|e| panic!("Logger initialization failed with {}", e));

    let mut zipfile = String::new();
    {
        let mut ap = ArgumentParser::new();
        ap.set_description("compresses all process binaries into a zip file");
        ap.refer(&mut zipfile).add_argument("zipfile", Store, "name of the destination zip file").required();
        ap.parse_args_or_exit();
    }

    
    let zipfile = PathBuf::from(zipfile);
    let binaries = get_process_binaries();
    match write_zip(zipfile, &binaries, log_buffer.clone()) {
        Err(why)    => error!("failed: {}", why),
        Ok(_)       => ()
    }
}

fn get_process_binaries() -> BinaryStatus {
    let sys = System::new_all();
    let mut binaries = BinaryStatus {
        files: HashSet::new()
    };
    for (_pid, process) in sys.get_processes() {
        let path = process.exe();

        if ! path.exists() {
            warn!("process {}({}) refers to invalid program name, omitting...", process.name(), process.pid());
            continue;
        }

        if ! binaries.files.contains(path) {
            binaries.files.insert(path.to_path_buf());
        }
    }
    binaries
}

fn write_zip(zipfile: PathBuf, binaries: &BinaryStatus, log_buffer: Arc<Mutex<Vec<u8>>>) -> ZipResult<()> {
    let path = std::path::Path::new(&zipfile);
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    
    let mut sha1_hashes = Builder::default();

    #[cfg(windows)]
    let re_drive = Regex::new(r"^(?P<p>[A-Za-z]):").unwrap();

    for p in &binaries.files {
        let mut f = match File::open(p) {
            Ok(f) => f,
            Err(why) => {
                error!("error while opening '{}': {}", p.to_str().unwrap(), why);
                continue;
            }
        };
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer)?;

        update_sha1_hashes(&mut sha1_hashes, &buffer, &p.to_str().unwrap());

        let pstr = match p.to_slash() {
            Some(v) => v,
            None    => return ZipResult::Err(ZipError::FileNotFound),
        };
        
        #[cfg(windows)]
        let pstr = match re_drive.replace_all(&pstr[..], "$p") {
            Cow::Borrowed(s)    => String::from(s),
            Cow::Owned(s)     => s,
        };

        info!("adding {}", pstr);
        zip.start_file(pstr, options)?;
        zip.write_all(&*buffer)?;
    }

    zip.start_file("messages.log", options)?;
    zip.write_all(log_buffer.lock().unwrap().deref())?;

    zip.start_file("sha1_hashes.csv", options)?;
    zip.write_all(sha1_hashes.string().unwrap().as_bytes())?;

    zip.finish()?;
    Result::Ok(())
}

fn update_sha1_hashes(builder: &mut Builder, buffer: impl AsRef<[u8]>, filename: &str) {
    let mut hasher = Sha1::new();
    hasher.update(&buffer);
    let result = hasher.finalize();
    builder.append(hex::encode(&result[..]));
    builder.append(";");
    builder.append(filename);
    builder.append("\n");
}
