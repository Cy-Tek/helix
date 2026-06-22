use helix_tui::{
    backend::{Backend, TestBackend},
    terminal::{MediaCommand, MediaImage, MediaOperation},
    Terminal,
};
use helix_view::graphics::Rect;

#[test]
fn terminal_buffer_size_should_not_be_limited() {
    let backend = TestBackend::new(400, 400);
    let terminal = Terminal::new(backend).unwrap();
    let size = terminal.backend().size().unwrap();
    assert_eq!(size.width, 400);
    assert_eq!(size.height, 400);
}

#[test]
fn terminal_records_media_operations() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let image = MediaImage {
        id: 7,
        area: Rect::new(10, 4, 20, 8),
        width: 160,
        height: 96,
        payload_hash: 42,
        png: vec![137, 80, 78, 71],
    };

    terminal
        .draw_media(&[MediaCommand::Image(image.clone())])
        .unwrap();

    assert_eq!(
        terminal.backend().media_operations(),
        &[MediaOperation::RenderImage(image.clone())]
    );

    terminal.draw_media(&[]).unwrap();

    assert_eq!(
        terminal.backend().media_operations(),
        &[
            MediaOperation::RenderImage(image),
            MediaOperation::ClearImage { id: 7 }
        ]
    );
}

// #[test]
// fn terminal_draw_returns_the_completed_frame() -> Result<(), Box<dyn Error>> {
//     let backend = TestBackend::new(10, 10);
//     let mut terminal = Terminal::new(backend)?;
//     let frame = terminal.draw(|f| {
//         let text = Text::from("Test");
//         let paragraph = Paragraph::new(&text);
//         f.render_widget(paragraph, f.size());
//     })?;
//     assert_eq!(frame.buffer.get(0, 0).symbol, "T");
//     assert_eq!(frame.area, Rect::new(0, 0, 10, 10));
//     terminal.backend_mut().resize(8, 8);
//     let frame = terminal.draw(|f| {
//         let text = Text::from("test");
//         let paragraph = Paragraph::new(&text);
//         f.render_widget(paragraph, f.size());
//     })?;
//     assert_eq!(frame.buffer.get(0, 0).symbol, "t");
//     assert_eq!(frame.area, Rect::new(0, 0, 8, 8));
//     Ok(())
// }
