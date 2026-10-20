//! Connects to a real VNC server. Run with `--features tokio -- --ignored --test-threads=1`,
//! JUSTVNC_TIGERVNC_ADDR pointing at the container in `interop/tigervnc` and
//! JUSTVNC_TIGERVNC_EXEC the command that runs a program inside it (e.g. `docker exec tigervnc`).
//! The tests share the server's clipboard and pointer, so they must not run in parallel.
#![cfg(feature = "tokio")]

use std::time::Duration;

use justvnc_client::tokio::{Connection, connect};
use justvnc_client::{ClientConfig, CutText, Event, Framebuffer, Rect};
use tokio::net::TcpStream;

async fn connect_to_tigervnc() -> Connection<TcpStream> {
    let addr = std::env::var("JUSTVNC_TIGERVNC_ADDR").expect("JUSTVNC_TIGERVNC_ADDR is not set");
    let stream = TcpStream::connect(addr).await.unwrap();
    connect(stream, ClientConfig::default()).await.unwrap()
}

/// Runs `command` with `sh -c` inside the TigerVNC container, on its X display.
fn exec_in_container(command: &str) -> Vec<u8> {
    let exec = std::env::var("JUSTVNC_TIGERVNC_EXEC").expect("JUSTVNC_TIGERVNC_EXEC is not set");
    let mut parts = exec.split_whitespace();
    let output = std::process::Command::new(parts.next().expect("empty JUSTVNC_TIGERVNC_EXEC"))
        .args(parts)
        .args(["sh", "-c", &format!("export DISPLAY=:0; {command}")])
        .output()
        .unwrap();
    assert!(output.status.success(), "{command}: {output:?}");
    output.stdout
}

async fn next_matching(
    connection: &mut Connection<TcpStream>,
    wanted: impl Fn(&Event) -> bool,
) -> Event {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = connection.next_event().await.unwrap();
            if wanted(&event) {
                return event;
            }
        }
    })
    .await
    .expect("event did not arrive in time")
}

#[tokio::test]
#[ignore = "needs a running TigerVNC; set JUSTVNC_TIGERVNC_ADDR"]
async fn connects_to_tigervnc_and_reports_its_desktop() {
    let addr = std::env::var("JUSTVNC_TIGERVNC_ADDR").expect("JUSTVNC_TIGERVNC_ADDR is not set");
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    let connection = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        connect(stream, ClientConfig::default()),
    )
    .await
    .expect("handshake timed out")
    .unwrap();

    let desktop = connection.desktop();
    assert_eq!((desktop.width, desktop.height), (1024, 768));
    assert_eq!(desktop.name.to_string_lossy(), "justvnc-test");
    assert_eq!(desktop.pixel_format.bits_per_pixel, 32);
}

/// The password the `interop/tigervnc` container is started with (VNC_PASSWORD) in CI.
const TIGERVNC_PASSWORD: &str = "justvnc-secret";

#[tokio::test]
#[ignore = "needs a running TigerVNC with VNC_PASSWORD; set JUSTVNC_TIGERVNC_AUTH_ADDR"]
async fn authenticates_to_tigervnc_with_a_password() {
    let addr =
        std::env::var("JUSTVNC_TIGERVNC_AUTH_ADDR").expect("JUSTVNC_TIGERVNC_AUTH_ADDR is not set");
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let config = ClientConfig {
        password: Some(TIGERVNC_PASSWORD.into()),
        ..ClientConfig::default()
    };

    let connection =
        tokio::time::timeout(std::time::Duration::from_secs(10), connect(stream, config))
            .await
            .expect("handshake timed out")
            .unwrap();

    assert_eq!(connection.desktop().name.to_string_lossy(), "justvnc-test");
}

/// Xvnc's `-retro` root window: a 4-pixel diagonal stipple, white where this holds.
fn retro_is_white(x: usize, y: usize) -> bool {
    x % 4 == [3, 1, 2, 0][y % 4]
}

#[tokio::test]
#[ignore = "needs a running TigerVNC started with XVNC_ARGS=-retro; set JUSTVNC_TIGERVNC_ADDR"]
async fn captures_a_frame_from_tigervnc() {
    let addr = std::env::var("JUSTVNC_TIGERVNC_ADDR").expect("JUSTVNC_TIGERVNC_ADDR is not set");
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();

    let frame = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut connection = connect(stream, ClientConfig::default()).await.unwrap();
        let desktop = connection.desktop().clone();
        let area = Rect {
            x: 0,
            y: 0,
            width: desktop.width,
            height: desktop.height,
        };
        connection.request_update(false, area).await.unwrap();
        let mut framebuffer =
            Framebuffer::new(desktop.width, desktop.height, connection.pixel_format());
        loop {
            let event = connection.next_event().await.unwrap();
            framebuffer.apply(&event);
            if event == Event::UpdateComplete {
                return framebuffer;
            }
        }
    })
    .await
    .expect("no complete update in time");

    // Rows 0..300 lie above the centred cursor; every pixel there follows the stipple.
    let (width, bpp) = (usize::from(frame.width()), 4);
    for y in 0..300 {
        for x in 0..width {
            let pixel = &frame.pixels()[(y * width + x) * bpp..][..bpp];
            let white = pixel[..3] == [0xff, 0xff, 0xff];
            assert_eq!(white, retro_is_white(x, y), "pixel ({x}, {y}) = {pixel:?}");
        }
    }
}

#[tokio::test]
#[ignore = "needs TigerVNC; set JUSTVNC_TIGERVNC_ADDR and JUSTVNC_TIGERVNC_EXEC"]
async fn sends_clipboard_text_tigervnc_takes() {
    let mut connection = connect_to_tigervnc().await;

    let text = CutText::from_text("justvnc caf\u{e9}\r\nline2").unwrap();
    connection.send_cut_text(&text).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // TigerVNC stores the Latin-1 text in the X clipboard as UTF-8.
    assert_eq!(
        exec_in_container("xsel -o -b"),
        "justvnc caf\u{e9}\nline2".as_bytes()
    );
}

#[tokio::test]
#[ignore = "needs TigerVNC; set JUSTVNC_TIGERVNC_ADDR and JUSTVNC_TIGERVNC_EXEC"]
async fn receives_clipboard_text_from_tigervnc() {
    let mut connection = connect_to_tigervnc().await;

    // printf's octal escapes are the UTF-8 bytes of é.
    exec_in_container(r"printf 'from X \303\251' | xsel -i -b");

    let event = next_matching(&mut connection, |e| matches!(e, Event::CutText(_))).await;
    let Event::CutText(text) = event else {
        unreachable!()
    };
    assert_eq!(text.to_string(), "from X \u{e9}");
}

#[tokio::test]
#[ignore = "needs TigerVNC; set JUSTVNC_TIGERVNC_ADDR and JUSTVNC_TIGERVNC_EXEC"]
async fn reports_tigervncs_bell() {
    let mut connection = connect_to_tigervnc().await;

    exec_in_container("xkbbell");

    next_matching(&mut connection, |e| *e == Event::Bell).await;
}

#[tokio::test]
#[ignore = "needs TigerVNC; set JUSTVNC_TIGERVNC_ADDR and JUSTVNC_TIGERVNC_EXEC"]
async fn sends_key_events_x_sees() {
    let mut connection = connect_to_tigervnc().await;
    exec_in_container(
        "rm -f /tmp/justvnc-xev.log; (timeout 5 xev -root -event keyboard > /tmp/justvnc-xev.log 2>&1 &)",
    );
    tokio::time::sleep(Duration::from_millis(700)).await;

    connection.key_event(true, 0x6a).await.unwrap();
    connection.key_event(false, 0x6a).await.unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;

    let log = String::from_utf8(exec_in_container("cat /tmp/justvnc-xev.log")).unwrap();
    let j = "keysym 0x6a, j";
    assert!(
        log.contains("KeyPress") && log.contains("KeyRelease"),
        "{log}"
    );
    assert_eq!(log.matches(j).count(), 2, "{log}");
}

/// Xvnc draws its 16x16 cursor into the framebuffer with its top-left 7 pixels above and left
/// of the pointer, because Cursor was not requested.
fn cursor_box(x: usize, y: usize) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    (x - 7..x + 9, y - 7..y + 9)
}

fn matches_stipple(
    frame: &Framebuffer,
    xs: std::ops::Range<usize>,
    ys: std::ops::Range<usize>,
) -> bool {
    let width = usize::from(frame.width());
    ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
        .all(|(x, y)| {
            let white = frame.pixels()[(y * width + x) * 4..][..3] == [0xff, 0xff, 0xff];
            white == retro_is_white(x, y)
        })
}

async fn read_update(connection: &mut Connection<TcpStream>, frame: &mut Framebuffer) {
    loop {
        let event = next_matching(connection, |_| true).await;
        frame.apply(&event);
        if event == Event::UpdateComplete {
            return;
        }
    }
}

/// Moves the pointer to (`x`, `y`) and reads updates until the cursor Xvnc draws is there and
/// nowhere near (`away_x`, `away_y`). Xvnc moves its cursor only once the X server has handled
/// the event, so the first update after a PointerEvent can still show the old position.
async fn move_pointer_until_drawn(
    connection: &mut Connection<TcpStream>,
    frame: &mut Framebuffer,
    (x, y): (u16, u16),
    (away_x, away_y): (u16, u16),
) {
    let area = Rect {
        x: 0,
        y: 0,
        width: frame.width(),
        height: frame.height(),
    };
    let (bx, by) = cursor_box(usize::from(x), usize::from(y));
    let (ax, ay) = cursor_box(usize::from(away_x), usize::from(away_y));
    connection.pointer_event(0, x, y).await.unwrap();
    for _ in 0..50 {
        // A full update, because an incremental one is never answered when nothing changed.
        connection.request_update(false, area).await.unwrap();
        read_update(connection, frame).await;
        if !matches_stipple(frame, bx.clone(), by.clone())
            && matches_stipple(frame, ax.clone(), ay.clone())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the cursor did not move to ({x}, {y})");
}

#[tokio::test]
#[ignore = "needs a running TigerVNC started with XVNC_ARGS=-retro; set JUSTVNC_TIGERVNC_ADDR"]
async fn moves_tigervncs_pointer() {
    let mut connection = connect_to_tigervnc().await;
    let desktop = connection.desktop().clone();
    let (width, height) = (desktop.width, desktop.height);
    let centre = (width / 2, height / 2);
    let mut frame = Framebuffer::new(width, height, connection.pixel_format());
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    connection.request_update(false, area).await.unwrap();
    read_update(&mut connection, &mut frame).await;
    move_pointer_until_drawn(&mut connection, &mut frame, centre, (200, 100)).await;

    move_pointer_until_drawn(&mut connection, &mut frame, (200, 100), centre).await;

    // Leave the pointer where a fresh Xvnc has it, so other tests see the cursor at the centre.
    move_pointer_until_drawn(&mut connection, &mut frame, centre, (200, 100)).await;
}
