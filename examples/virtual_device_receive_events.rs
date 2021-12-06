// Create a virtual keyboard, just while this is running.
// Generally this requires root.

use evdev::uinput::VirtualDevice;
use evdev::{uinput::VirtualDeviceBuilder, AttributeSet, EventType, InputEvent};
use evdev::{Device, LedType};

use nix::sys::epoll;
use std::os::unix::io::{AsRawFd, RawFd};

fn main() -> std::io::Result<()> {
    let mut leds = AttributeSet::<LedType>::new();
    leds.insert(LedType::LED_CAPSL);
    leds.insert(LedType::LED_NUML);

    let mut device = VirtualDeviceBuilder::new()?
        .name("Fake Keyboard")
        .with_leds(&leds)?
        .build()
        .unwrap();

    let device_event_node = get_event_node(&device);
    println!("Virtual device located at {}", device_event_node);

    // Set up epoll on the "driver" side
    // See example evtest_nonblocking for explanations
    let epoll_fd = Epoll::new(epoll::epoll_create1(
        epoll::EpollCreateFlags::EPOLL_CLOEXEC,
    )?);
    let mut event = epoll::EpollEvent::new(epoll::EpollFlags::EPOLLIN, 0);
    epoll::epoll_ctl(
        epoll_fd.as_raw_fd(),
        epoll::EpollOp::EpollCtlAdd,
        device.as_raw_fd(),
        Some(&mut event),
    )?;

    // Wait a moment until the virtual device is initialized, otherwise kernel will return permission denied
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Now, we spawn a thread for the client side, which will send LED events to our virtual device.
    let handle: std::thread::JoinHandle<Result<(), std::io::Error>> =
        std::thread::spawn(move || {
            let mut client = Device::open(&device_event_node)?;
            client.send_events(&[InputEvent::new(
                EventType::LED,
                LedType::LED_NUML.0,
                i32::MAX,
            )])?;

            std::thread::sleep(std::time::Duration::from_secs(1));

            client.send_events(&[InputEvent::new(
                EventType::LED,
                LedType::LED_CAPSL.0,
                i32::MAX,
            )])?;
            std::thread::sleep(std::time::Duration::from_secs(1));

            client.send_events(&[InputEvent::new(EventType::LED, LedType::LED_NUML.0, 0)])?;

            std::thread::sleep(std::time::Duration::from_secs(1));

            client.send_events(&[InputEvent::new(EventType::LED, LedType::LED_CAPSL.0, 0)])?;

            Ok(())
        });

    // We start listening for events on the driver side.
    // In our case, we just print the client requests instead of actually turning LEDs off and on
    let mut events = [epoll::EpollEvent::empty(); 2];

    'outer: loop {
        match device.fetch_events() {
            Ok(iterator) => {
                for ev in iterator {
                    println!("Received event from client: {:?}", ev);
                    // We exit after the last client event
                    if ev.code() == LedType::LED_CAPSL.0 && ev.value() == 0 {
                        break 'outer;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Wait forever for bytes available on raw_fd
                epoll::epoll_wait(epoll_fd.as_raw_fd(), &mut events, -1)?;
            }
            Err(e) => {
                eprintln!("{}", e);
                break;
            }
        }
    }
    handle.join().unwrap().ok();
    Ok(())
}

fn get_event_node(device: &VirtualDevice) -> String {
    // Yes, this is ugly right now, but that's hopefully ok for an example
    // Feel free to improve
    let sysname = device.get_sysname().unwrap();
    let sys_path = format!("/sys/devices/virtual/input/{}", sysname);
    let event_node_name = std::fs::read_dir(sys_path)
        .and_then(|mut read_dir| {
            let entry = read_dir
                .find(|dir_entry| {
                    let dir_entry = dir_entry.as_ref().unwrap();
                    dir_entry
                        .file_name()
                        .into_string()
                        .unwrap()
                        .starts_with("event")
                })
                .unwrap()
                .unwrap();
            Ok(entry.file_name().into_string().unwrap())
        })
        .unwrap();
    format!("/dev/input/{}", event_node_name)
}

// The rest here is to ensure the epoll handle is cleaned up properly.
// You can also use the epoll crate, if you prefer.
struct Epoll(RawFd);

impl Epoll {
    pub(crate) fn new(fd: RawFd) -> Self {
        Epoll(fd)
    }
}

impl AsRawFd for Epoll {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl Drop for Epoll {
    fn drop(&mut self) {
        let _ = nix::unistd::close(self.0);
    }
}
