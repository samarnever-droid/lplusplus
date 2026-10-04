//! Host-backed networking builtins for the rewrite runtime.
//!
//! The frozen v1 ABI already exposes `net_*`/`http_*` symbols. The rewrite
//! runtime keeps that ABI intact and implements the symbols as thin wrappers
//! over the host TCP/UDP stack (`std::net`), passing opaque `i64` handles back
//! to L++ programs. Strings remain ARC-owned NUL-terminated payloads, matching
//! the rest of the runtime.

use libc::{c_char, strlen};
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::string::arc_string;

/// The bytes of a NUL-terminated C string (empty for NULL), without the
/// terminator. Kept local to avoid exposing string.rs' private helper.
unsafe fn cstr_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(s.cast::<u8>(), strlen(s)) }
}

unsafe fn arc_empty() -> *mut c_char {
    unsafe { arc_string(&[]) }
}

fn cstr_lossy(s: *const c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let bytes = unsafe { cstr_bytes(s) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn port_u16(port: i64) -> Option<u16> {
    if (0..=65_535).contains(&port) {
        Some(port as u16)
    } else {
        None
    }
}

fn duration_ms(milliseconds: i64) -> Option<Duration> {
    if milliseconds < 0 {
        None
    } else {
        Some(Duration::from_millis(milliseconds as u64))
    }
}

enum Socket {
    Listener(TcpListener),
    Stream(TcpStream),
    Udp(UdpSocket),
}

enum SocketClone {
    Listener(TcpListener),
    Stream(TcpStream),
    Udp(UdpSocket),
}

impl Socket {
    fn try_clone_socket(&self) -> std::io::Result<SocketClone> {
        match self {
            Socket::Listener(listener) => listener.try_clone().map(SocketClone::Listener),
            Socket::Stream(stream) => stream.try_clone().map(SocketClone::Stream),
            Socket::Udp(socket) => socket.try_clone().map(SocketClone::Udp),
        }
    }
}

struct Slot {
    generation: u32,
    socket: Option<Socket>,
}

struct SocketTable {
    slots: Vec<Slot>,
}

impl SocketTable {
    fn new() -> Self {
        Self { slots: Vec::new() }
    }

    fn encode(index: usize, generation: u32) -> i64 {
        // generation starts at 1; keep the high bit clear so handles are
        // positive for practical process lifetimes.
        let generation_bits = u64::from(generation & 0x7fff_ffff);
        ((generation_bits << 32) | ((index as u64) + 1)) as i64
    }

    fn decode(handle: i64) -> Option<(usize, u32)> {
        if handle <= 0 {
            return None;
        }
        let raw = handle as u64;
        let index1 = (raw & 0xffff_ffff) as usize;
        let generation = ((raw >> 32) & 0x7fff_ffff) as u32;
        if index1 == 0 || generation == 0 {
            return None;
        }
        Some((index1 - 1, generation))
    }

    fn insert(&mut self, socket: Socket) -> i64 {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.socket.is_none() {
                if slot.generation == 0 {
                    slot.generation = 1;
                }
                slot.socket = Some(socket);
                return Self::encode(index, slot.generation);
            }
        }
        self.slots.push(Slot {
            generation: 1,
            socket: Some(socket),
        });
        Self::encode(self.slots.len() - 1, 1)
    }

    fn clone_handle(&self, handle: i64) -> Option<SocketClone> {
        let (index, generation) = Self::decode(handle)?;
        let slot = self.slots.get(index)?;
        if slot.generation != generation {
            return None;
        }
        slot.socket.as_ref()?.try_clone_socket().ok()
    }

    fn remove(&mut self, handle: i64) -> Option<Socket> {
        let (index, generation) = Self::decode(handle)?;
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        let socket = slot.socket.take();
        slot.generation = slot.generation.wrapping_add(1) & 0x7fff_ffff;
        if slot.generation == 0 {
            slot.generation = 1;
        }
        socket
    }
}

fn socket_table() -> &'static Mutex<SocketTable> {
    static SOCKETS: OnceLock<Mutex<SocketTable>> = OnceLock::new();
    SOCKETS.get_or_init(|| Mutex::new(SocketTable::new()))
}

fn store_socket(socket: Socket) -> i64 {
    socket_table()
        .lock()
        .map(|mut table| table.insert(socket))
        .unwrap_or(0)
}

fn clone_socket(handle: i64) -> Option<SocketClone> {
    socket_table()
        .lock()
        .ok()
        .and_then(|table| table.clone_handle(handle))
}

fn take_socket(handle: i64) -> Option<Socket> {
    socket_table()
        .lock()
        .ok()
        .and_then(|mut table| table.remove(handle))
}

fn connect_tcp(host: &str, port: u16, timeout_ms: Option<i64>) -> Option<TcpStream> {
    let addrs = (host, port).to_socket_addrs().ok()?;
    let timeout = timeout_ms.filter(|ms| *ms > 0).and_then(duration_ms);
    for addr in addrs {
        let result = if let Some(timeout) = timeout {
            TcpStream::connect_timeout(&addr, timeout)
        } else {
            TcpStream::connect(addr)
        };
        if let Ok(stream) = result {
            let _ = stream.set_nodelay(true);
            if let Some(timeout) = timeout {
                let _ = stream.set_read_timeout(Some(timeout));
                let _ = stream.set_write_timeout(Some(timeout));
            }
            return Some(stream);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn bind_tcp_listener(port: u16) -> std::io::Result<TcpListener> {
    use std::mem::size_of;
    use std::os::fd::FromRawFd;

    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let yes: libc::c_int = 1;
        let _ = libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&yes as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        );

        let addr = libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr { s_addr: 0 },
            sin_zero: [0; 8],
        };
        let rc = libc::bind(
            fd,
            (&addr as *const libc::sockaddr_in).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_in>() as libc::socklen_t,
        );
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }
        if libc::listen(fd, 16) != 0 {
            let err = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(err);
        }
        Ok(TcpListener::from_raw_fd(fd))
    }
}

#[cfg(not(target_os = "linux"))]
fn bind_tcp_listener(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("0.0.0.0", port))
}

fn set_socket_timeout(socket: &SocketClone, read_ms: Option<i64>, write_ms: Option<i64>) -> bool {
    let read = read_ms.and_then(duration_ms);
    let write = write_ms.and_then(duration_ms);
    let mut ok = true;
    match socket {
        SocketClone::Stream(stream) => {
            if read_ms.is_some() && stream.set_read_timeout(read).is_err() {
                ok = false;
            }
            if write_ms.is_some() && stream.set_write_timeout(write).is_err() {
                ok = false;
            }
        }
        SocketClone::Udp(socket) => {
            if read_ms.is_some() && socket.set_read_timeout(read).is_err() {
                ok = false;
            }
            if write_ms.is_some() && socket.set_write_timeout(write).is_err() {
                ok = false;
            }
        }
        // std::net::TcpListener has no portable timeout API; accept-timeout is
        // implemented by `lpp_net_accept_timeout` via temporary nonblocking mode.
        SocketClone::Listener(_) => {}
    }
    ok
}

fn set_socket_nonblocking(socket: &SocketClone, enable: bool) -> bool {
    match socket {
        SocketClone::Listener(listener) => listener.set_nonblocking(enable).is_ok(),
        SocketClone::Stream(stream) => stream.set_nonblocking(enable).is_ok(),
        SocketClone::Udp(socket) => socket.set_nonblocking(enable).is_ok(),
    }
}

#[cfg(unix)]
fn raw_fd(socket: &SocketClone) -> std::os::fd::RawFd {
    use std::os::fd::AsRawFd;
    match socket {
        SocketClone::Listener(listener) => listener.as_raw_fd(),
        SocketClone::Stream(stream) => stream.as_raw_fd(),
        SocketClone::Udp(socket) => socket.as_raw_fd(),
    }
}

fn poll_socket(socket: &SocketClone, timeout_ms: i64) -> i64 {
    #[cfg(unix)]
    unsafe {
        let timeout = if timeout_ms < 0 {
            -1
        } else {
            timeout_ms.min(i64::from(i32::MAX)) as libc::c_int
        };
        let mut pfd = libc::pollfd {
            fd: raw_fd(socket),
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = libc::poll(&mut pfd as *mut libc::pollfd, 1, timeout);
        if rc > 0 && (pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR)) != 0 {
            return 1;
        }
        return 0;
    }

    #[cfg(not(unix))]
    {
        let _ = (socket, timeout_ms);
        0
    }
}

fn set_keepalive(socket: &SocketClone, enable: bool) -> bool {
    #[cfg(unix)]
    unsafe {
        let value: libc::c_int = if enable { 1 } else { 0 };
        return libc::setsockopt(
            raw_fd(socket),
            libc::SOL_SOCKET,
            libc::SO_KEEPALIVE,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        ) == 0;
    }

    #[cfg(not(unix))]
    {
        let _ = (socket, enable);
        true
    }
}

/// `net_connect(host, port) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_connect(host: *const c_char, port: i64) -> i64 {
    let Some(host) = cstr_lossy(host) else {
        return 0;
    };
    let Some(port) = port_u16(port) else {
        return 0;
    };
    match connect_tcp(&host, port, None) {
        Some(stream) => store_socket(Socket::Stream(stream)),
        None => 0,
    }
}

/// `net_dial(host, port, timeout_ms) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_dial(host: *const c_char, port: i64, timeout_ms: i64) -> i64 {
    let Some(host) = cstr_lossy(host) else {
        return 0;
    };
    let Some(port) = port_u16(port) else {
        return 0;
    };
    match connect_tcp(&host, port, Some(timeout_ms)) {
        Some(stream) => store_socket(Socket::Stream(stream)),
        None => 0,
    }
}

/// `net_listen(port) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_listen(port: i64) -> i64 {
    let Some(port) = port_u16(port) else {
        return 0;
    };
    match bind_tcp_listener(port) {
        Ok(listener) => store_socket(Socket::Listener(listener)),
        Err(_) => 0,
    }
}

/// `net_accept(listener) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_accept(listener: i64) -> i64 {
    let Some(SocketClone::Listener(listener)) = clone_socket(listener) else {
        return 0;
    };
    match listener.accept() {
        Ok((stream, _)) => {
            let _ = stream.set_nodelay(true);
            store_socket(Socket::Stream(stream))
        }
        Err(_) => 0,
    }
}

/// `net_accept_timeout(listener, timeout_ms) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_accept_timeout(listener: i64, timeout_ms: i64) -> i64 {
    let Some(SocketClone::Listener(listener)) = clone_socket(listener) else {
        return 0;
    };
    if timeout_ms <= 0 {
        return match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                store_socket(Socket::Stream(stream))
            }
            Err(_) => 0,
        };
    }

    if listener.set_nonblocking(true).is_err() {
        return 0;
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = listener.set_nonblocking(false);
                let _ = stream.set_nodelay(true);
                return store_socket(Socket::Stream(stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    let _ = listener.set_nonblocking(false);
                    return 0;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => {
                let _ = listener.set_nonblocking(false);
                return 0;
            }
        }
    }
}

fn send_bytes(handle: i64, data: &[u8]) -> i64 {
    let Some(socket) = clone_socket(handle) else {
        return -1;
    };
    match socket {
        SocketClone::Stream(mut stream) => {
            if stream.write_all(data).is_ok() {
                data.len() as i64
            } else {
                -1
            }
        }
        SocketClone::Udp(socket) => match socket.send(data) {
            Ok(n) => n as i64,
            Err(_) => -1,
        },
        SocketClone::Listener(_) => -1,
    }
}

/// `net_send_all(handle, data) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_send_all(handle: i64, data: *const c_char) -> i64 {
    if data.is_null() {
        return -1;
    }
    send_bytes(handle, unsafe { cstr_bytes(data) })
}

/// `net_send(handle, data) -> Int` — complete-write semantics, matching v1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_send(handle: i64, data: *const c_char) -> i64 {
    unsafe { lpp_net_send_all(handle, data) }
}

/// `net_recv(handle, max_bytes) -> Str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_recv(handle: i64, max_bytes: i64) -> *mut c_char {
    if max_bytes <= 0 {
        return unsafe { arc_empty() };
    }
    let Ok(size) = usize::try_from(max_bytes) else {
        return unsafe { arc_empty() };
    };
    let mut buf = Vec::new();
    if buf.try_reserve_exact(size).is_err() {
        return unsafe { arc_empty() };
    }
    buf.resize(size, 0);

    let Some(socket) = clone_socket(handle) else {
        return unsafe { arc_empty() };
    };
    let received = match socket {
        SocketClone::Stream(mut stream) => stream.read(&mut buf).unwrap_or(0),
        SocketClone::Udp(socket) => socket.recv(&mut buf).unwrap_or(0),
        SocketClone::Listener(_) => 0,
    };
    unsafe { arc_string(&buf[..received]) }
}

/// `net_recv_udp(handle, max_bytes) -> Str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_recv_udp(handle: i64, max_bytes: i64) -> *mut c_char {
    unsafe { lpp_net_recv(handle, max_bytes) }
}

/// `net_close(handle)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_close(handle: i64) {
    if let Some(socket) = take_socket(handle) {
        if let Socket::Stream(stream) = socket {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

/// `net_set_timeout(handle, milliseconds) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_set_timeout(handle: i64, milliseconds: i64) -> i64 {
    if milliseconds <= 0 {
        return 0;
    }
    let Some(socket) = clone_socket(handle) else {
        return 0;
    };
    if set_socket_timeout(&socket, Some(milliseconds), Some(milliseconds)) {
        1
    } else {
        0
    }
}

/// `net_set_deadline(handle, read_ms, write_ms) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_set_deadline(handle: i64, read_ms: i64, write_ms: i64) -> i64 {
    let Some(socket) = clone_socket(handle) else {
        return 0;
    };
    let read = (read_ms >= 0).then_some(read_ms);
    let write = (write_ms >= 0).then_some(write_ms);
    if set_socket_timeout(&socket, read, write) {
        1
    } else {
        0
    }
}

/// `net_set_nonblocking(handle, enable) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_set_nonblocking(handle: i64, enable: i64) -> i64 {
    let Some(socket) = clone_socket(handle) else {
        return 0;
    };
    if set_socket_nonblocking(&socket, enable != 0) {
        1
    } else {
        0
    }
}

/// `net_poll(handle, timeout_ms) -> Int` — readiness for reading/accepting.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_poll(handle: i64, timeout_ms: i64) -> i64 {
    let Some(socket) = clone_socket(handle) else {
        return 0;
    };
    poll_socket(&socket, timeout_ms)
}

/// `net_set_keepalive(handle, enable, idle_s, interval_s, count) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_set_keepalive(
    handle: i64,
    enable: i64,
    _idle_s: i64,
    _interval_s: i64,
    _count: i64,
) -> i64 {
    let Some(socket) = clone_socket(handle) else {
        return 0;
    };
    if set_keepalive(&socket, enable != 0) {
        1
    } else {
        0
    }
}

/// `net_dial_udp(host, port, timeout_ms) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_dial_udp(host: *const c_char, port: i64, timeout_ms: i64) -> i64 {
    let Some(host) = cstr_lossy(host) else {
        return 0;
    };
    let Some(port) = port_u16(port) else {
        return 0;
    };
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", 0)) else {
        return 0;
    };
    let Ok(mut addrs) = (host.as_str(), port).to_socket_addrs() else {
        return 0;
    };
    let Some(addr) = addrs.next() else {
        return 0;
    };
    if socket.connect(addr).is_err() {
        return 0;
    }
    if timeout_ms > 0 {
        if let Some(timeout) = duration_ms(timeout_ms) {
            let _ = socket.set_read_timeout(Some(timeout));
            let _ = socket.set_write_timeout(Some(timeout));
        }
    }
    store_socket(Socket::Udp(socket))
}

/// `net_listen_udp(port) -> Int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_listen_udp(port: i64) -> i64 {
    let Some(port) = port_u16(port) else {
        return 0;
    };
    match UdpSocket::bind(("0.0.0.0", port)) {
        Ok(socket) => store_socket(Socket::Udp(socket)),
        Err(_) => 0,
    }
}

/// `net_resolve(host) -> Str` — first IPv4 address when available, else the
/// first resolved address, else the empty string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_resolve(host: *const c_char) -> *mut c_char {
    let Some(host) = cstr_lossy(host) else {
        return unsafe { arc_empty() };
    };
    if host.is_empty() {
        return unsafe { arc_empty() };
    }
    let Ok(addrs) = (host.as_str(), 0u16).to_socket_addrs() else {
        return unsafe { arc_empty() };
    };
    let mut first: Option<IpAddr> = None;
    for addr in addrs {
        let ip = addr.ip();
        if first.is_none() {
            first = Some(ip);
        }
        if ip.is_ipv4() {
            return unsafe { arc_string(ip.to_string().as_bytes()) };
        }
    }
    match first {
        Some(ip) => unsafe { arc_string(ip.to_string().as_bytes()) },
        None => unsafe { arc_empty() },
    }
}

fn parse_http_url(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port.parse::<u16>().ok()?;
            (host.to_owned(), port)
        }
        _ => (authority.to_owned(), 80),
    };
    Some((host, port, path.to_owned()))
}

fn http_round_trip(host: &str, port: u16, request: &[u8], timeout_ms: i64) -> Vec<u8> {
    let Some(mut stream) = connect_tcp(host, port, Some(timeout_ms)) else {
        return Vec::new();
    };
    if stream.write_all(request).is_err() {
        return Vec::new();
    }
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&chunk[..n]);
                // v1 reads at most 64 KiB for the HTTP helpers; keep the same
                // bounded behaviour so a peer cannot make the runtime grow
                // memory without limit.
                if response.len() >= 65_536 {
                    break;
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => return Vec::new(),
        }
    }
    if let Some(index) = response.windows(4).position(|w| w == b"\r\n\r\n") {
        response[(index + 4)..].to_vec()
    } else {
        response
    }
}

/// `http_get(url, timeout_ms) -> Str` — minimal HTTP/1.1 over the `net_*`
/// primitives; HTTPS is intentionally outside this ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_http_get(url: *const c_char, timeout_ms: i64) -> *mut c_char {
    let Some(url) = cstr_lossy(url) else {
        return unsafe { arc_empty() };
    };
    let Some((host, port, path)) = parse_http_url(&url) else {
        return unsafe { arc_empty() };
    };
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/rewrite\r\n\r\n"
    );
    let body = http_round_trip(&host, port, request.as_bytes(), timeout_ms);
    unsafe { arc_string(&body) }
}

/// `http_post(url, data, content_type, timeout_ms) -> Str`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_http_post(
    url: *const c_char,
    data: *const c_char,
    content_type: *const c_char,
    timeout_ms: i64,
) -> *mut c_char {
    let Some(url) = cstr_lossy(url) else {
        return unsafe { arc_empty() };
    };
    let Some((host, port, path)) = parse_http_url(&url) else {
        return unsafe { arc_empty() };
    };
    let data = unsafe { cstr_bytes(data) };
    let content_type = if content_type.is_null() {
        "application/x-www-form-urlencoded".to_owned()
    } else {
        String::from_utf8_lossy(unsafe { cstr_bytes(content_type) }).into_owned()
    };

    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/rewrite\r\n\r\n",
        data.len()
    )
    .into_bytes();
    request.extend_from_slice(data);
    let body = http_round_trip(&host, port, &request, timeout_ms);
    unsafe { arc_string(&body) }
}
