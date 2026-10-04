//! L++ native networking runtime.
//!
//! This is an ABI boundary, not a Rust API leaked into L++: all entry points
//! use stable C representations and opaque integer handles. The initial layer
//! intentionally uses `std::net` only. It is a correct, blocking TCP core that
//! can later sit under a mio/kqueue/epoll/IOCP reactor without changing L++
//! source-level names.
//!
//! It never invokes cURL, a command-line client, or a foreign networking
//! process. TLS and HTTP are intentionally not claimed by this crate yet.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const INVALID_HANDLE: i64 = 0;
const MAX_READ_BYTES: usize = 16 * 1024 * 1024;

enum Socket {
    Stream(TcpStream),
    Listener(TcpListener),
    Datagram(UdpSocket),
}

struct Registry {
    next_handle: i64,
    sockets: HashMap<i64, Arc<Mutex<Socket>>>,
}

impl Registry {
    fn new() -> Self {
        Self {
            next_handle: 1,
            sockets: HashMap::new(),
        }
    }

    fn insert(&mut self, socket: Socket) -> i64 {
        // Never recycle handles. A stale L++ handle cannot accidentally close
        // or write to a different connection after a close/open race.
        let handle = self.next_handle;
        if handle <= 0 || handle == i64::MAX {
            return INVALID_HANDLE;
        }
        self.next_handle += 1;
        self.sockets.insert(handle, Arc::new(Mutex::new(socket)));
        handle
    }
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::new()))
}

/// Gets a socket without holding the global handle-table lock across network
/// I/O. Each connection has its own lock; a blocked read on one connection
/// cannot stall unrelated connections, listener registration, or close.
fn socket_for(handle: i64) -> Option<Arc<Mutex<Socket>>> {
    registry().lock().ok()?.sockets.get(&handle).cloned()
}

fn host_from_ptr(host: *const c_char) -> Option<String> {
    if host.is_null() {
        return None;
    }
    // L++ strings are NUL-terminated in the existing runtime ABI. Invalid
    // UTF-8 is rejected rather than passed through ambiguously to DNS.
    // SAFETY: every public caller is required to pass a valid, NUL-terminated
    // L++ string pointer; null was rejected above.
    unsafe { CStr::from_ptr(host) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

fn duration(milliseconds: i64) -> Option<Duration> {
    (milliseconds > 0).then(|| Duration::from_millis(milliseconds as u64))
}

/// Open a TCP connection after DNS resolution. Returns 0 on any failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_rs_connect(
    host: *const c_char,
    port: i64,
    timeout_ms: i64,
) -> i64 {
    if !(1..=65535).contains(&port) {
        return INVALID_HANDLE;
    }
    let Some(host) = host_from_ptr(host) else {
        return INVALID_HANDLE;
    };
    let timeout = duration(timeout_ms).unwrap_or(Duration::from_secs(30));
    let Ok(addrs) = (host.as_str(), port as u16).to_socket_addrs() else {
        return INVALID_HANDLE;
    };

    // Try every resolved address just as a production client should: DNS can
    // return IPv6 and IPv4 endpoints with different reachability.
    let stream = addrs
        .filter_map(|address| TcpStream::connect_timeout(&address, timeout).ok())
        .next();
    let Some(stream) = stream else {
        return INVALID_HANDLE;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    registry()
        .lock()
        .ok()
        .map(|mut table| table.insert(Socket::Stream(stream)))
        .unwrap_or(0)
}

/// Bind an IPv4/IPv6-capable TCP listener. Returns 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_listen(port: i64) -> i64 {
    if !(1..=65535).contains(&port) {
        return INVALID_HANDLE;
    }
    let Ok(listener) = TcpListener::bind(("0.0.0.0", port as u16)) else {
        return INVALID_HANDLE;
    };
    registry()
        .lock()
        .ok()
        .map(|mut table| table.insert(Socket::Listener(listener)))
        .unwrap_or(0)
}

/// Accept one connection. The stream is registered only after accept succeeds.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_accept(listener: i64) -> i64 {
    // Clone the OS listener while holding the registry briefly, then release
    // the lock before a potentially blocking accept. One stalled listener must
    // never prevent independent sockets from reading, writing, or closing.
    let Some(socket) = socket_for(listener) else {
        return INVALID_HANDLE;
    };
    let listener = {
        let guard = match socket.lock() {
            Ok(guard) => guard,
            Err(_) => return INVALID_HANDLE,
        };
        let Socket::Listener(listener) = &*guard else {
            return INVALID_HANDLE;
        };
        match listener.try_clone() {
            Ok(listener) => listener,
            Err(_) => return INVALID_HANDLE,
        }
    };
    let Ok((stream, _peer)) = listener.accept() else {
        return INVALID_HANDLE;
    };
    registry()
        .lock()
        .ok()
        .map(|mut table| table.insert(Socket::Stream(stream)))
        .unwrap_or(0)
}

/// Write all bytes in `data`. Returns the byte count or -1 on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_rs_send_all(handle: i64, data: *const c_char) -> i64 {
    if data.is_null() {
        return -1;
    }
    // SAFETY: public ABI requires a valid NUL-terminated L++ string pointer.
    let bytes = unsafe { CStr::from_ptr(data) }.to_bytes();
    let Some(socket) = socket_for(handle) else {
        return -1;
    };
    let mut guard = match socket.lock() {
        Ok(guard) => guard,
        Err(_) => return -1,
    };
    let Socket::Stream(stream) = &mut *guard else {
        return -1;
    };
    match stream.write_all(bytes).and_then(|_| stream.flush()) {
        Ok(()) => i64::try_from(bytes.len()).unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Set both read and write deadlines. Returns 1 on success, 0 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_set_timeout(handle: i64, milliseconds: i64) -> i64 {
    let Some(timeout) = duration(milliseconds) else {
        return 0;
    };
    let Some(socket) = socket_for(handle) else {
        return 0;
    };
    let guard = match socket.lock() {
        Ok(guard) => guard,
        Err(_) => return 0,
    };
    let configured = match &*guard {
        Socket::Stream(stream) => {
            stream.set_read_timeout(Some(timeout)).is_ok()
                && stream.set_write_timeout(Some(timeout)).is_ok()
        }
        Socket::Datagram(socket) => {
            socket.set_read_timeout(Some(timeout)).is_ok()
                && socket.set_write_timeout(Some(timeout)).is_ok()
        }
        Socket::Listener(_) => false,
    };
    configured as i64
}

/// Read up to `max_bytes`, returning an owned C string. The caller must release
/// it with `lpp_net_rs_free_string`, never libc `free`.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_recv(handle: i64, max_bytes: i64) -> *mut c_char {
    let length = match usize::try_from(max_bytes) {
        Ok(length) if length <= MAX_READ_BYTES => length,
        _ => return ptr::null_mut(),
    };
    let mut bytes = vec![0_u8; length];
    let Some(socket) = socket_for(handle) else {
        return ptr::null_mut();
    };
    let mut guard = match socket.lock() {
        Ok(guard) => guard,
        Err(_) => return ptr::null_mut(),
    };
    let Socket::Stream(stream) = &mut *guard else {
        return ptr::null_mut();
    };
    let Ok(read) = stream.read(&mut bytes) else {
        return CString::default().into_raw();
    };
    bytes.truncate(read);
    // L++'s current Str ABI cannot represent embedded NUL bytes. Return null
    // rather than silently truncate a binary protocol payload.
    CString::new(bytes)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// Bind a UDP endpoint. Port zero is accepted here so callers can request an
/// OS-selected ephemeral port; TCP's public listen API remains explicit.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_udp_bind(port: i64) -> i64 {
    if !(0..=65535).contains(&port) {
        return INVALID_HANDLE;
    }
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", port as u16)) else {
        return INVALID_HANDLE;
    };
    registry()
        .lock()
        .ok()
        .map(|mut table| table.insert(Socket::Datagram(socket)))
        .unwrap_or(0)
}

/// Connects an existing UDP endpoint to a peer after DNS resolution. UDP
/// connect performs no handshake; it selects a default peer and lets send/recv
/// use the same ownership and deadline rules as TCP.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_rs_udp_connect(
    handle: i64,
    host: *const c_char,
    port: i64,
) -> i64 {
    if !(1..=65535).contains(&port) {
        return 0;
    }
    let Some(host) = host_from_ptr(host) else {
        return 0;
    };
    let Ok(addrs) = (host.as_str(), port as u16).to_socket_addrs() else {
        return 0;
    };
    let Some(socket) = socket_for(handle) else {
        return 0;
    };
    let guard = match socket.lock() {
        Ok(guard) => guard,
        Err(_) => return 0,
    };
    let Socket::Datagram(socket) = &*guard else {
        return 0;
    };
    for address in addrs {
        if socket.connect(address).is_ok() {
            return 1;
        }
    }
    0
}

/// Sends a complete datagram to the connected peer. Datagram writes are atomic
/// at the OS API boundary: a short successful write is rejected as failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_rs_udp_send(handle: i64, data: *const c_char) -> i64 {
    if data.is_null() {
        return -1;
    }
    let bytes = unsafe { CStr::from_ptr(data) }.to_bytes();
    let Some(socket) = socket_for(handle) else {
        return -1;
    };
    let guard = match socket.lock() {
        Ok(guard) => guard,
        Err(_) => return -1,
    };
    let Socket::Datagram(socket) = &*guard else {
        return -1;
    };
    match socket.send(bytes) {
        Ok(sent) if sent == bytes.len() => i64::try_from(sent).unwrap_or(-1),
        _ => -1,
    }
}

/// Receives one connected UDP datagram as a Rust-owned C string. Binary
/// datagrams containing NUL are rejected until L++ gains a byte-buffer ABI.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_udp_recv(handle: i64, max_bytes: i64) -> *mut c_char {
    let length = match usize::try_from(max_bytes) {
        Ok(length) if length <= MAX_READ_BYTES => length,
        _ => return ptr::null_mut(),
    };
    let mut bytes = vec![0_u8; length];
    let Some(socket) = socket_for(handle) else {
        return ptr::null_mut();
    };
    let guard = match socket.lock() {
        Ok(guard) => guard,
        Err(_) => return ptr::null_mut(),
    };
    let Socket::Datagram(socket) = &*guard else {
        return ptr::null_mut();
    };
    let Ok(read) = socket.recv(&mut bytes) else {
        return CString::default().into_raw();
    };
    bytes.truncate(read);
    CString::new(bytes)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// Close a socket or listener. It is idempotent; stale handles do nothing.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_net_rs_close(handle: i64) {
    if let Ok(mut table) = registry().lock() {
        table.sockets.remove(&handle);
    }
}

/// Release a string returned by `lpp_net_rs_recv` using Rust's allocator.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_net_rs_free_string(value: *mut c_char) {
    if !value.is_null() {
        // SAFETY: only a pointer previously returned by lpp_net_rs_recv may
        // enter this function, so Rust owns the allocation and its layout.
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_arguments_fail_without_creating_handles() {
        assert_eq!(unsafe { lpp_net_rs_connect(ptr::null(), 80, 10) }, 0);
        assert_eq!(lpp_net_rs_listen(0), 0);
        assert_eq!(lpp_net_rs_set_timeout(999_999, 10), 0);
        assert_eq!(unsafe { lpp_net_rs_send_all(999_999, ptr::null()) }, -1);
    }

    #[test]
    fn ffi_tcp_round_trip_completes_a_protocol_write() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
        let port = listener.local_addr().expect("address").port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).expect("request");
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").expect("response");
        });

        let host = CString::new("127.0.0.1").expect("host");
        let request = CString::new("ping").expect("request");
        let handle = unsafe { lpp_net_rs_connect(host.as_ptr(), i64::from(port), 1_000) };
        assert_ne!(handle, 0);
        assert_eq!(lpp_net_rs_set_timeout(handle, 1_000), 1);
        assert_eq!(unsafe { lpp_net_rs_send_all(handle, request.as_ptr()) }, 4);
        let reply = lpp_net_rs_recv(handle, 32);
        assert!(!reply.is_null());
        // SAFETY: recv returns a NUL-terminated allocation owned by this ABI.
        assert_eq!(unsafe { CStr::from_ptr(reply) }.to_bytes(), b"pong");
        unsafe { lpp_net_rs_free_string(reply) };
        lpp_net_rs_close(handle);
        server.join().expect("server thread");
    }

    #[test]
    fn ffi_udp_round_trip_preserves_one_datagram() {
        use std::net::UdpSocket;
        use std::thread;

        let peer = UdpSocket::bind(("127.0.0.1", 0)).expect("udp peer");
        let port = peer.local_addr().expect("address").port();
        let server = thread::spawn(move || {
            let mut request = [0_u8; 16];
            let (count, sender) = peer.recv_from(&mut request).expect("datagram");
            assert_eq!(&request[..count], b"ping");
            peer.send_to(b"pong", sender).expect("reply");
        });

        let socket = lpp_net_rs_udp_bind(0);
        assert_ne!(socket, 0);
        let host = CString::new("127.0.0.1").expect("host");
        let request = CString::new("ping").expect("request");
        assert_eq!(
            unsafe { lpp_net_rs_udp_connect(socket, host.as_ptr(), i64::from(port)) },
            1
        );
        assert_eq!(lpp_net_rs_set_timeout(socket, 1_000), 1);
        assert_eq!(unsafe { lpp_net_rs_udp_send(socket, request.as_ptr()) }, 4);
        let reply = lpp_net_rs_udp_recv(socket, 32);
        assert!(!reply.is_null());
        assert_eq!(unsafe { CStr::from_ptr(reply) }.to_bytes(), b"pong");
        unsafe { lpp_net_rs_free_string(reply) };
        lpp_net_rs_close(socket);
        server.join().expect("server thread");
    }
}
