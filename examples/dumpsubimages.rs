//
// Copyright (c) 2026 Ivan Rodnov.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions
// are met:
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
// ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHOR OR CONTRIBUTORS BE LIABLE
// FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
// OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
// HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
// LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
// OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
// SUCH DAMAGE.
//

//! What images are embedded in a file, and where.
//!
//! The counterpart to `dumpexif` for [`exif::Exif::embedded_images`]. A raw carries
//! a full-size JPEG preview, and its ADDRESS is metadata — so a viewer can put
//! a raw on screen by copying those bytes, with no demosaic and no raw
//! decoder. This prints what the library can find, and verifies each one is
//! really a JPEG rather than trusting the offset.
//!
//!     cargo run --features "make_note mpf" --example dumpsubimages -- FILE...
//!
//! `--extract DIR` writes each image out.

use std::io::{Read, Seek, SeekFrom};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let extract = args.iter().position(|a| a == "--extract").map(|i| {
        let dir = args.get(i + 1).cloned().unwrap_or_else(|| ".".to_owned());
        args.drain(i..=i + 1);
        dir
    });

    for path in &args {
        let Ok(file) = std::fs::File::open(path) else {
            println!("{path}: cannot open");
            continue;
        };
        let mut reader = std::io::BufReader::new(&file);
        let exif = match exif::Reader::new().read_from_container(&mut reader) {
            Ok(e) => e,
            Err(e) => {
                println!("{}: {e}", short(path));
                continue;
            }
        };
        let images = exif.embedded_images();
        println!("{}  ({} embedded image(s))", short(path), images.len());
        for (i, img) in images.iter().enumerate() {
            // An offset is only useful if the bytes there are actually an
            // image, so check the SOI rather than reporting the number.
            let mut soi = [0u8; 2];
            let ok = reader
                .seek(SeekFrom::Start(img.offset))
                .and_then(|_| reader.read_exact(&mut soi))
                .is_ok()
                && soi == [0xFF, 0xD8];
            let dims = img
                .dimensions(&mut reader)
                .map_or_else(|_| "?".to_owned(), |(w, h)| format!("{w}x{h}"));
            // A CFA image is the undemosaiced sensor mosaic, addressed exactly
            // like a preview and not a picture at all — saying so is the point.
            let kind = match (img.photometric, img.compression) {
                (Some(32803), _) => "CFA mosaic".to_owned(),
                (_, Some(1)) => "uncompressed".to_owned(),
                _ if ok => "JPEG".to_owned(),
                (_, Some(c)) => format!("compression {c}"),
                _ => "unknown".to_owned(),
            };
            // Bit 0 of NewSubfileType: a reduced-resolution version of another
            // image, i.e. exactly what a viewer wants and not the raw itself.
            let role = match img.subfile_type {
                Some(t) if t & 1 == 1 => "preview",
                Some(_) => "full-res",
                None => "-",
            };
            println!(
                "  [{i}] {:<10} offset={:<10} {:>9} bytes  {:<11} {:<9} {}",
                img.source.name(),
                img.offset,
                img.length,
                dims,
                role,
                kind,
            );
            if let Some(dir) = &extract {
                let name = format!("{}/{}_{i}.jpg", dir, stem(path));
                match img.save_to_file(&mut reader, &name) {
                    Ok(n) => println!("        -> {name} ({n} bytes)"),
                    Err(e) => println!("        !! {e}"),
                }
            }
        }
    }
}

fn short(p: &str) -> String {
    std::path::Path::new(p)
        .file_name()
        .map_or_else(|| p.to_owned(), |s| s.to_string_lossy().into_owned())
        .chars()
        .take(44)
        .collect()
}

fn stem(p: &str) -> String {
    std::path::Path::new(p)
        .file_stem()
        .map_or_else(|| "img".to_owned(), |s| s.to_string_lossy().into_owned())
}
