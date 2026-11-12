//! Explicit native clipboard acceptance on a disposable display, never part of ordinary headless tests.
#![allow(clippy::unwrap_used)]
use vw_platform::clipboard::ClipboardService;
use vw_platform::clipboard_content::{ClipboardContent, ClipboardFormat};
#[tokio::test]
#[ignore = "requires a private disposable desktop and VW_CLIPBOARD_NATIVE_TEST=1"]
async fn html_and_png_round_trip_on_the_native_clipboard() {
    assert_eq!(std::env::var("VW_CLIPBOARD_NATIVE_TEST").unwrap(), "1");
    let service = ClipboardService::new().unwrap();
    let wanted = ClipboardContent::Html("<b>native clipboard</b><i> detail</i>".into());
    assert!(service.write_content(wanted.clone()).await.unwrap() == wanted);
    assert!(service.read_content(ClipboardFormat::Html).await.unwrap() == wanted);
    let image = image::RgbaImage::from_pixel(3, 2, image::Rgba([25, 100, 220, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let actual = service
        .write_content(ClipboardContent::Png(encoded.into_inner()))
        .await
        .unwrap();
    let ClipboardContent::Png(bytes) = actual else {
        panic!("PNG readback missing")
    };
    assert_eq!(image::load_from_memory(&bytes).unwrap().into_rgba8(), image);
    assert!(matches!(
        service.read_content(ClipboardFormat::Png).await.unwrap(),
        ClipboardContent::Png(_)
    ));
    service.shutdown().await;
}
