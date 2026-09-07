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
// Dump the parsed MakerNote fields of each file, the counterpart to
// `dumpexif` for the `make_note` feature. Without it a MakerNote prints as one
// opaque hex blob, which says nothing about whether the vendor parser ran.
//
//     cargo run --features make_note --example dumpmakernote -- FILE...
//
// A tag may be given to print just that one, in hex: `-- -t 0x1113 FILE...`.

#[cfg(feature = "make_note")]
fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut want: Option<u16> = None;
    if args.peek().map(String::as_str) == Some("-t") {
        args.next();
        let raw = args.next().expect("-t needs a tag");
        let hex = raw.trim_start_matches("0x");
        want = Some(u16::from_str_radix(hex, 16).expect("tag must be hex"));
    }
    for path in args {
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let exif = match exif::Reader::new()
            .read_from_container(&mut std::io::BufReader::new(&file))
        {
            Ok(e) => e,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let vendor = match exif.maker_note_vendor() {
            Ok(v) => format!("{v:?}"),
            Err(e) => format!("unrecognised ({e})"),
        };
        println!("{path}\n  vendor: {vendor}");
        let mut n = 0usize;
        for f in exif.maker_note_fields() {
            n += 1;
            if want.is_some_and(|w| w != f.tag.number()) {
                continue;
            }
            println!("  {:#06x} {}: {}", f.tag.number(), f.tag, f.display_value());
        }
        println!("  {n} maker note fields");
    }
}

#[cfg(not(feature = "make_note"))]
fn main() {
    println!("rebuild with --features make_note");
}
