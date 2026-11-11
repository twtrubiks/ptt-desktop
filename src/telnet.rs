//! telnet-over-WebSocket 下行濾層（移植自原專案 `src/telnet.ts`）：
//! 1. 丟棄串流最前面的 `HTTP/1.1 200 OK\r\n\r\n` 前導（PTT 閘道行為）；
//!    只在開頭確實是 "HTTP/" 時才丟棄，否則把已收位元組回放進一般處理，不吞資料
//! 2. 濾掉 telnet IAC 協商序列（只濾不回應，實測可行）
//!
//! IAC 序列可能被切在兩個 frame 之間，因此濾層必須有狀態。

const IAC: u8 = 0xff;
const SE: u8 = 0xf0;
const SB: u8 = 0xfa;
const WILL: u8 = 0xfb;
const WONT: u8 = 0xfc;
const DO: u8 = 0xfd;
const DONT: u8 = 0xfe;

const PREAMBLE_PREFIX: &[u8] = b"HTTP/";
const PREAMBLE_END: &[u8] = b"\r\n\r\n";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// 比對串流開頭是否為 "HTTP/"，不符則回放已收位元組
    Prefix,
    /// 已確認 HTTP 前導，丟棄直到讀完 \r\n\r\n
    Preamble,
    /// 一般資料
    Data,
    /// 已讀到 IAC，等指令 byte
    Iac,
    /// WILL/WONT/DO/DONT 之後，等選項 byte
    Option,
    /// SB 子協商內容
    Sub,
    /// 子協商內讀到 IAC，等 SE
    SubIac,
}

pub struct TelnetFilter {
    state: State,
    prefix_held: Vec<u8>,
    preamble_matched: usize,
}

impl Default for TelnetFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl TelnetFilter {
    pub fn new() -> Self {
        Self {
            state: State::Prefix,
            prefix_held: Vec::new(),
            preamble_matched: 0,
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Prefix;
        self.prefix_held.clear();
        self.preamble_matched = 0;
    }

    pub fn filter(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(chunk.len());
        for &byte in chunk {
            self.step(byte, &mut out);
        }
        out
    }

    fn step(&mut self, byte: u8, out: &mut Vec<u8>) {
        match self.state {
            State::Prefix => {
                if byte == PREAMBLE_PREFIX[self.prefix_held.len()] {
                    self.prefix_held.push(byte);
                    if self.prefix_held.len() == PREAMBLE_PREFIX.len() {
                        self.prefix_held.clear();
                        self.state = State::Preamble;
                    }
                    return;
                }
                // 開頭不是 HTTP 前導：已收位元組與當前 byte 回放進一般 telnet 處理
                self.state = State::Data;
                let held = std::mem::take(&mut self.prefix_held);
                for h in held {
                    self.step(h, out);
                }
                self.step(byte, out);
            }
            State::Preamble => {
                if byte == PREAMBLE_END[self.preamble_matched] {
                    self.preamble_matched += 1;
                    if self.preamble_matched == PREAMBLE_END.len() {
                        self.state = State::Data;
                    }
                } else {
                    self.preamble_matched = usize::from(byte == 0x0d);
                }
            }
            State::Data => {
                if byte == IAC {
                    self.state = State::Iac;
                } else {
                    out.push(byte);
                }
            }
            State::Iac => {
                if byte == IAC {
                    out.push(IAC); // IAC IAC = 資料中的 0xFF
                    self.state = State::Data;
                } else if byte == SB {
                    self.state = State::Sub;
                } else if byte == WILL || byte == WONT || byte == DO || byte == DONT {
                    self.state = State::Option;
                } else {
                    self.state = State::Data; // NOP 等單 byte 指令，丟棄
                }
            }
            State::Option => self.state = State::Data, // 吃掉選項 byte
            State::Sub => {
                if byte == IAC {
                    self.state = State::SubIac;
                }
            }
            State::SubIac => {
                self.state = if byte == SE { State::Data } else { State::Sub };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREAMBLE: &[u8] = b"HTTP/1.1 200 OK\r\n\r\n";

    fn fresh() -> TelnetFilter {
        let mut f = TelnetFilter::new();
        f.filter(PREAMBLE);
        f
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    #[test]
    fn preamble_dropped_data_kept() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(&cat(&[PREAMBLE, b"AB"])), b"AB");
    }

    #[test]
    fn preamble_split_across_frames() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(&PREAMBLE[..10]), b"");
        assert_eq!(f.filter(&cat(&[&PREAMBLE[10..], b"A"])), b"A");
    }

    #[test]
    fn preamble_end_split_in_middle() {
        let mut f = TelnetFilter::new();
        let cut = PREAMBLE.len() - 2;
        assert_eq!(f.filter(&PREAMBLE[..cut]), b"");
        assert_eq!(f.filter(&cat(&[&PREAMBLE[cut..], b"A"])), b"A");
    }

    #[test]
    fn no_preamble_iac_first() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(&[IAC, DO, 0x18, 0x41]), b"A");
    }

    #[test]
    fn no_preamble_plain_data_not_swallowed() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(b"ABC"), b"ABC");
    }

    #[test]
    fn partial_prefix_then_mismatch_replays() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(b"HTTX"), b"HTTX");
    }

    #[test]
    fn partial_prefix_split_then_mismatch_replays() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(b"HT"), b"");
        assert_eq!(f.filter(b"A"), b"HTA");
    }

    #[test]
    fn replayed_bytes_with_iac_still_filtered() {
        let mut f = TelnetFilter::new();
        assert_eq!(f.filter(&[0x48, IAC, DO, 0x18, 0x41]), b"HA");
    }

    #[test]
    fn do_will_filtered() {
        let mut f = fresh();
        assert_eq!(
            f.filter(&[IAC, DO, 0x18, 0x41, IAC, WILL, 0x01, 0x42]),
            b"AB"
        );
    }

    #[test]
    fn iac_split_across_frames() {
        let mut f = fresh();
        assert_eq!(f.filter(&[0x41, IAC]), b"A");
        assert_eq!(f.filter(&[DO]), b"");
        assert_eq!(f.filter(&[0x18, 0x42]), b"B");
    }

    #[test]
    fn subnegotiation_filtered() {
        let mut f = fresh();
        assert_eq!(f.filter(&[IAC, SB, 0x18, 0x01, IAC, SE, 0x41]), b"A");
    }

    #[test]
    fn subnegotiation_split_across_frames() {
        let mut f = fresh();
        assert_eq!(f.filter(&[IAC, SB, 0x18]), b"");
        assert_eq!(f.filter(&[0x01, IAC]), b"");
        assert_eq!(f.filter(&[SE, 0x41]), b"A");
    }

    #[test]
    fn iac_iac_is_data_ff() {
        let mut f = fresh();
        assert_eq!(f.filter(&[0x41, IAC, IAC, 0x42]), &[0x41, 0xff, 0x42]);
    }

    #[test]
    fn single_byte_command_dropped() {
        let mut f = fresh();
        assert_eq!(f.filter(&[IAC, 0xf1, 0x41]), b"A");
    }

    #[test]
    fn reset_waits_for_preamble_again() {
        let mut f = fresh();
        f.filter(b"A");
        f.reset();
        assert_eq!(f.filter(&cat(&[PREAMBLE, b"B"])), b"B");
    }
}
