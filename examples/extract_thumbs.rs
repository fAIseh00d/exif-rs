use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <image_file>", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);

    println!("Reading image: {}", file_path);
    println!();

    // Parse Exif data
    let exif = exif::Reader::new().read_from_container(&mut reader)?;

    // Get embedded images from Exif
    println!("=== Exif Embedded Images ===");
    let thumbnails = exif.thumbnails();
    if thumbnails.is_empty() {
        println!("No embedded images found in Exif data");
    } else {
        for (i, img) in thumbnails.iter().enumerate() {
            println!(
                "Image {}: {} - {} bytes at offset {}",
                i + 1,
                img.source.name(),
                img.length,
                img.offset
            );
        }
    }
    println!();

    // Extract and save Exif thumbnails
    if !thumbnails.is_empty() {
        println!("Extracting Exif thumbnails...");
        for (i, img) in thumbnails.iter().enumerate() {
            let output_path = format!("{}_{}_exif.jpg", file_path, i + 1);

            // Note: The offset in ImageInfo from Exif.thumbnails() is relative to the
            // TIFF data start, not the file start. For a complete implementation,
            // you'd need to track the APP1 segment offset in JPEG files.
            println!("  Would save to: {} ({} bytes)", output_path, img.length);
        }
        println!();
    }

    // Try to get MPF info (requires mpf feature)
    #[cfg(feature = "mpf")]
    {
        println!("=== MPF (Multi-Picture Format) Images ===");

        // Need to reopen the file for MPF parsing
        let file = File::open(file_path)?;
        let mut reader = BufReader::new(file);

        match exif::get_mpf_info(&mut reader)? {
            Some(mpf) => {
                println!("MPF Version: {}", mpf.version);
                println!("Number of images: {}", mpf.number_of_images);
                println!();

                for (i, img) in mpf.images.iter().enumerate() {
                    println!("MPF Image {}:", i + 1);
                    println!("  Type: 0x{:x}", img.r#type);
                    println!(
                        "  Length: {} bytes ({:.2} KB)",
                        img.length,
                        img.length as f64 / 1024.0
                    );
                    println!("  Offset: {}", img.offset);
                }
                println!();

                // Extract and save preview image if available
                if let Some(preview) = mpf.get_preview_image() {
                    println!("Extracting MPF preview image...");

                    match preview.extract_data(&mut reader) {
                        Ok(data) => {
                            let output_path = format!("{}_mpf_preview.jpg", file_path);
                            std::fs::write(&output_path, &data)?;
                            println!("  Saved to: {} ({} bytes)", output_path, data.len());
                        }
                        Err(e) => {
                            eprintln!("  Error extracting MPF preview: {}", e);
                        }
                    }
                } else {
                    println!("No MPF preview image found");
                }
            }
            None => {
                println!("No MPF data found");
            }
        }
        println!();
    }

    #[cfg(not(feature = "mpf"))]
    {
        println!("=== MPF Support ===");
        println!("MPF feature is not enabled. Rebuild with --features mpf to enable.");
        println!();
    }

    // Display MakerNote vendor info
    #[cfg(feature = "make_note")]
    {
        println!("=== MakerNote Info ===");
        match exif.maker_note_vendor() {
            Ok(vendor) => {
                println!("Vendor: {:?}", vendor);

                // MakerNote preview images are already included in exif.thumbnails()
                // when the make_note feature is enabled
                let maker_note_images: Vec<_> = thumbnails
                    .iter()
                    .filter(|img| {
                        matches!(
                            img.source,
                            exif::EmbeddedSubImageSource::MakerNotePreview1
                                | exif::EmbeddedSubImageSource::MakerNotePreview2
                                | exif::EmbeddedSubImageSource::MakerNotePreview3
                        )
                    })
                    .collect();

                if !maker_note_images.is_empty() {
                    println!("MakerNote preview images:");
                    for img in maker_note_images {
                        println!(
                            "  {}: {} bytes at offset {}",
                            img.source.name(),
                            img.length,
                            img.offset
                        );
                    }
                } else {
                    println!("No MakerNote preview images found");
                }
            }
            Err(e) => {
                println!("MakerNote not found or not supported: {:?}", e);
            }
        }
        println!();
    }

    #[cfg(not(feature = "make_note"))]
    {
        println!("=== MakerNote Support ===");
        println!("MakerNote feature is not enabled. Rebuild with --features make_note to enable.");
        println!();
    }

    // Summary based on enabled features
    println!("=== Feature Configuration ===");
    println!("mpf: {}", cfg!(feature = "mpf"));
    println!("make_note: {}", cfg!(feature = "make_note"));
    println!();
    println!("Feature combinations:");
    println!("- mpf=false, make_note=false: Exif thumbnail only");
    println!("- mpf=true,  make_note=false: Exif thumbnail + MPF images");
    println!("- mpf=false, make_note=true:  Exif thumbnail + MakerNote images");
    println!("- mpf=true,  make_note=true:  Exif thumbnail + MPF + MakerNote images");

    Ok(())
}
