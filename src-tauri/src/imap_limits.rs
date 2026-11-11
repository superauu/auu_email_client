//! Reject oversized wire data before imap's buffering / literal allocation.
//! A rejected connection stays poisoned; partial responses must never be reused.
use std::io::{self, Read, Write};
#[cfg(test)]
#[path = "imap_limits_tests.rs"]
mod tests;

pub(crate) const MAX_LITERAL: usize = 16 * 1024 * 1024;
const MAX_RESPONSE: usize = 32 * 1024 * 1024;
const MAX_LINE: usize = 1024 * 1024;
const MAX_LINES: usize = 10_000;
const MAX_SEARCH_UIDS: usize = 50_000;

#[derive(Default)]
struct Budget {
    bytes: usize,
    lines: usize,
    search_uids: usize,
    literal: usize,
    literal_response: bool,
    line: Vec<u8>,
}
impl Budget {
    fn receive(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        self.bytes = self.bytes.checked_add(bytes.len()).ok_or_else(limit)?;
        if self.bytes > MAX_RESPONSE {
            return Err(limit());
        }
        while !bytes.is_empty() {
            if self.literal > 0 {
                let n = self.literal.min(bytes.len());
                self.literal -= n;
                bytes = &bytes[n..];
                continue;
            }
            let n = bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| i + 1);
            if self.line.len() + n > MAX_LINE {
                return Err(limit());
            }
            self.line.extend_from_slice(&bytes[..n]);
            bytes = &bytes[n..];
            if !self.line.ends_with(b"\n") {
                continue;
            }
            self.lines += 1;
            if self.lines > MAX_LINES {
                return Err(limit());
            }
            let line = self.line.strip_suffix(b"\r\n").ok_or_else(limit)?;
            if !self.literal_response {
                let mut words = line
                    .split(u8::is_ascii_whitespace)
                    .filter(|s| !s.is_empty());
                if words.next() == Some(b"*".as_slice()) {
                    if let Some(kind) = words.next() {
                        self.literal_response =
                            [b"LIST".as_slice(), b"LSUB", b"STATUS", b"NAMESPACE", b"ID"]
                                .iter()
                                .any(|name| kind.eq_ignore_ascii_case(name))
                                || (kind.iter().all(u8::is_ascii_digit)
                                    && words
                                        .next()
                                        .is_some_and(|name| name.eq_ignore_ascii_case(b"FETCH")));
                    }
                }
            }
            if line
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"* SEARCH"))
                && line.get(8).is_none_or(u8::is_ascii_whitespace)
            {
                self.search_uids += line[8..]
                    .split(u8::is_ascii_whitespace)
                    .filter(|s| !s.is_empty())
                    .count();
                if self.search_uids > MAX_SEARCH_UIDS {
                    return Err(limit());
                }
            }
            let mut has_literal = false;
            if let Some(prefix) = line.strip_suffix(b"}").filter(|_| self.literal_response) {
                if let Some(start) = prefix.iter().rposition(|b| *b == b'{') {
                    let value = &prefix[start + 1..];
                    let value = value.strip_suffix(b"+").unwrap_or(value);
                    // Decimal overflow is an invalid declaration, never "no literal".
                    if !value.is_empty() && value.iter().all(u8::is_ascii_digit) {
                        let size = value
                            .iter()
                            .try_fold(0usize, |n, b| {
                                n.checked_mul(10)?.checked_add((b - b'0') as usize)
                            })
                            .ok_or_else(limit)?;
                        if size > MAX_LITERAL || size > MAX_RESPONSE - self.bytes {
                            return Err(limit());
                        }
                        self.literal = size;
                        has_literal = true;
                    }
                }
            }
            self.literal_response = has_literal;
            self.line.clear();
        }
        Ok(())
    }
}
#[derive(Debug)]
struct WireLimit;
impl std::fmt::Display for WireLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IMAP response exceeded safe wire limits")
    }
}
impl std::error::Error for WireLimit {}
fn limit() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, WireLimit)
}
pub(crate) fn network(error: imap::Error, context: &str) -> crate::error::AppError {
    if let imap::Error::Io(ref e) = error {
        if let Some(interrupted) = e
            .get_ref()
            .and_then(|e| e.downcast_ref::<crate::sync_control::Interrupted>())
        {
            return if interrupted.0 {
                crate::error::AppError {
                    code: "cancelled",
                    message: "同步已停止，已提交批次保留".into(),
                    retryable: false,
                }
            } else {
                crate::error::AppError::network("本轮同步达到时间预算，已提交批次保留")
            };
        }
    }
    if matches!(error, imap::Error::Io(ref e) if e.get_ref().is_some_and(|e| e.is::<WireLimit>())) {
        crate::error::AppError::network(format!(
            "{context}：IMAP 响应超过安全限额，连接已停止；本机数据与已提交进度保留。"
        ))
    } else {
        crate::error::AppError::network(context)
    }
}

pub(crate) struct LimitedStream<S> {
    inner: S,
    budget: Budget,
    wrote: bool,
    poisoned: bool,
    control: ControlSwitch,
    started: std::time::Instant,
    command_state: u8,
}
pub(crate) type ControlSwitch =
    std::sync::Arc<std::sync::Mutex<Option<crate::sync_control::Control>>>;
impl<S> LimitedStream<S> {
    pub(crate) fn control_handle(&self) -> ControlSwitch {
        self.control.clone()
    }
    fn check_control(&self) -> io::Result<()> {
        let control = self
            .control
            .lock()
            .map_err(|_| io::Error::other("IMAP control unavailable"))?;
        crate::sync_control::io_check(control.as_ref())
    }
    pub(crate) fn new(inner: S) -> Self {
        Self {
            inner,
            budget: Budget::default(),
            wrote: false,
            poisoned: false,
            control: std::sync::Arc::new(std::sync::Mutex::new(crate::sync_control::current())),
            started: std::time::Instant::now(),
            command_state: 0,
        }
    }
}
impl<S: Read> Read for LimitedStream<S> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.check_control()?;
        if self.started.elapsed() >= std::time::Duration::from_secs(60) {
            self.poisoned = true;
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                crate::sync_control::Interrupted(false),
            ));
        }
        if self.poisoned {
            return Err(limit());
        }
        // Even a caller-supplied huge buffer cannot cause a huge upstream read.
        let size = bytes.len().min(8192);
        let n = match self.inner.read(&mut bytes[..size]) {
            Ok(n) => n,
            Err(error) => {
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) && (!self.budget.line.is_empty() || self.budget.literal > 0)
                {
                    self.poisoned = true;
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "IMAP timeout inside a partial response",
                    ));
                }
                return Err(error);
            }
        };
        self.check_control()?;
        if self.started.elapsed() >= std::time::Duration::from_secs(60) {
            self.poisoned = true;
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                crate::sync_control::Interrupted(false),
            ));
        }
        if let Err(error) = self.budget.receive(&bytes[..n]) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(n)
    }
}
impl<S: Write> Write for LimitedStream<S> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check_control()?;
        if self.poisoned {
            return Err(limit());
        }
        let n = self.inner.write(bytes)?;
        for byte in &bytes[..n] {
            self.command_state = match self.command_state {
                0 if *byte == b'a' => 1,
                1 if byte.is_ascii_digit() => 2,
                2 if byte.is_ascii_digit() => 2,
                2 if *byte == b' ' => 3,
                3 => 3,
                _ => 4,
            };
        }
        self.wrote |= n > 0;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check_control()?;
        if self.poisoned {
            return Err(limit());
        }
        self.inner.flush()?;
        // imap 2.x flushes each command before reading its response. Empty flushes
        // don't replenish the budget; no command text or credentials are retained.
        if self.wrote {
            if self.command_state == 3 {
                self.started = std::time::Instant::now();
                self.budget.bytes = 0;
                self.budget.lines = 0;
                self.budget.search_uids = 0;
            }
            self.command_state = 0;
            self.wrote = false;
        }
        Ok(())
    }
}
impl<S: imap::extensions::idle::SetReadTimeout> imap::extensions::idle::SetReadTimeout
    for LimitedStream<S>
{
    fn set_read_timeout(
        &mut self,
        timeout: Option<std::time::Duration>,
    ) -> imap::error::Result<()> {
        self.inner.set_read_timeout(Some(
            timeout
                .unwrap_or(std::time::Duration::from_secs(10))
                .min(std::time::Duration::from_secs(10))
                .max(std::time::Duration::from_millis(1)),
        ))
    }
}
