// NOTE: Originally adapted from the `grep_json_deserialize` crate.
// See: https://github.com/Avi-D-coder/grep_json_deserialize/blob/master/src/lib.rs

use std::ffi::OsString;
use std::fmt::{self, Display};
use std::ops::Range;
use std::path::PathBuf;

use anyhow::Result;
use base64_simd::STANDARD as base64;
use serde::{Deserialize, Serialize};

/// A helper to easily select the `RgMessage` kind.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RgMessageKind {
    Begin,
    End,
    Match,
    Context,
    Summary,
}

/// A struct used to deserialise JSON values produced by `ripgrep`.
/// See: https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html
#[derive(Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type", content = "data")]
pub enum RgMessage<'a> {
    /// As specified in: [message-begin](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#message-begin).
    Begin { 
        #[serde(borrow)]
        path: ArbitraryData<'a>
    },
    /// As specified in: [message-end](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#message-end).
    End {
        #[serde(borrow)]
        path: ArbitraryData<'a>,
        binary_offset: Option<usize>,
        stats: Stats,
    },
    /// As specified in: [message-match](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#message-match).
    Match {
        #[serde(borrow)]
        path: ArbitraryData<'a>,
        lines: ArbitraryData<'a>,
        line_number: Option<usize>,
        absolute_offset: usize,
        submatches: Vec<SubMatch<'a>>,
    },
    /// As specified in: [message-context](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#message-context).
    Context {
        #[serde(borrow)]
        path: ArbitraryData<'a>,
        lines: ArbitraryData<'a>,
        line_number: Option<usize>,
        absolute_offset: usize,
        submatches: Vec<SubMatch<'a>>,
    },
    Summary {
        elapsed_total: Duration,
        stats: Stats,
    },
}

/// As specified in: [object-arbitrary-data](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#object-arbitrary-data).
#[derive(Deserialize, Debug, PartialEq, Eq, Clone, Hash)]
#[serde(untagged)]
pub enum ArbitraryData<'a> {
    Text { text: &'a str },
    Base64 { bytes: &'a str },
}

impl<'a> ArbitraryData<'a> {
    pub fn to_vec(&self) -> Vec<u8> {
        match self {
            ArbitraryData::Text { text } => text.as_bytes().to_vec(),
            ArbitraryData::Base64 { bytes } => base64.decode_to_vec(bytes).unwrap(),
        }
    }

    /// Converts to an `OsString`.
    #[cfg(unix)]
    pub fn to_os_string(&self) -> Result<OsString> {
        /// Convert Base64 encoded data to an OsString on Unix platforms.
        /// https://doc.rust-lang.org/std/ffi/index.html#on-unix
        use std::os::unix::ffi::OsStringExt;

        Ok(match self {
            ArbitraryData::Text { text } => OsString::from(text),
            ArbitraryData::Base64 { .. } => OsString::from_vec(self.to_vec()),
        })
    }

    /// Converts to an `OsString`.
    #[cfg(windows)]
    pub fn to_os_string(&self) -> Result<OsString> {
        /// Convert Base64 encoded data to an OsString on Windows platforms.
        /// https://doc.rust-lang.org/std/ffi/index.html#on-windows
        use std::os::windows::ffi::OsStringExt;

        Ok(match self {
            ArbitraryData::Text { text } => OsString::from(text),
            ArbitraryData::Base64 { .. } => {
                // Transmute decoded Base64 bytes as UTF-16 since that's what underlying paths are on Windows.
                let bytes_u16 = safe_transmute::transmute_vec::<u8, u16>(self.to_vec())
                    .or_else(|e| e.copy())?;

                OsString::from_wide(&bytes_u16)
            }
        })
    }

    pub fn to_path_buf(&self) -> Result<PathBuf> {
        self.to_os_string().map(PathBuf::from)
    }

    pub fn lossy_utf8(&self) -> String {
        match self {
            ArbitraryData::Text { text } => text.to_string(),
            ArbitraryData::Base64 { bytes } => {
                String::from_utf8_lossy(base64.decode_to_vec(bytes).unwrap().as_slice()).to_string()
            }
        }
    }
}

impl<'a> Display for ArbitraryData<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.lossy_utf8())
    }
}

/// As specified in: [object-stats](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#object-stats).
#[derive(Deserialize, Serialize, Debug, PartialEq, Eq, Clone)]
pub struct Stats {
    pub elapsed: Duration,
    pub searches: usize,
    pub searches_with_match: usize,
    pub bytes_searched: usize,
    pub bytes_printed: usize,
    pub matched_lines: usize,
    pub matches: usize,
}

/// As specified in: [object-duration](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#object-duration).
#[derive(Deserialize, Serialize, Debug, PartialEq, Eq, Clone)]
pub struct Duration {
    pub secs: usize,
    pub nanos: usize,
    pub human: String,
}

/// Almost as specified in: [object-submatch](https://docs.rs/grep-printer/0.1.5/grep_printer/struct.JSON.html#object-submatch).
/// `match` is deserialized to `text` because a rust reserves match as a keyword.
#[derive(Deserialize, Debug, PartialEq, Eq, Clone)]
#[serde(rename = "submatch")]
pub struct SubMatch<'a> {
    #[serde(rename = "match", borrow)]
    pub text: ArbitraryData<'a>,
    #[serde(flatten)]
    pub range: Range<usize>,
}

/// Utilities for tests.
#[cfg(test)]
#[allow(dead_code)]
pub mod test_utilities {
    use super::*;

    pub const RG_JSON_BEGIN: &str =
        r#"{"type":"begin","data":{"path":{"text":"src/model/item.rs"}}}"#;
    pub const RG_JSON_MATCH: &str = r#"{"type":"match","data":{"path":{"text":"src/model/item.rs"},"lines":{"text":"    Item::new(rg_msg)\n"},"line_number":197,"absolute_offset":5522,"submatches":[{"match":{"text":"Item"},"start":4,"end":8},{"match":{"text":"rg_msg"},"start":14,"end":20}]}}"#;
    pub const RG_JSON_MATCH_75_LONG: &str = r#"{"type":"match","data":{"path":{"text":"src/model/item.rs"},"lines":{"text":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxbar"},"line_number":1,"absolute_offset":5522,"submatches":[{"match":{"text":"bar"},"start":75,"end":78}]}}"#;
    pub const RG_JSON_CONTEXT_75_LONG: &str = r#"{"type":"context","data":{"path":{"text":"src/model/item.rs"},"lines":{"text":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"},"line_number":1,"absolute_offset":5522,"submatches":[]}}"#;
    pub const RG_JSON_CONTEXT: &str = r#"{"type":"context","data":{"path":{"text":"src/model/item.rs"},"lines":{"text":"  }\n"},"line_number":198,"absolute_offset":5544,"submatches":[]}}"#;
    pub const RG_JSON_CONTEXT_EMPTY: &str = r#"{"type":"context","data":{"path":{"text":"src/model/item.rs"},"lines":{"text":"\n"},"line_number":198,"absolute_offset":5544,"submatches":[]}}"#;
    pub const RG_JSON_END: &str = r#"{"type":"end","data":{"path":{"text":"src/model/item.rs"},"binary_offset":null,"stats":{"elapsed":{"secs":0,"nanos":97924,"human":"0.000098s"},"searches":1,"searches_with_match":1,"bytes_searched":5956,"bytes_printed":674,"matched_lines":2,"matches":2}}}"#;
    pub const RG_JSON_SUMMARY: &str = r#"{"data":{"elapsed_total":{"human":"0.013911s","nanos":13911027,"secs":0},"stats":{"bytes_printed":3248,"bytes_searched":18789,"elapsed":{"human":"0.000260s","nanos":260276,"secs":0},"matched_lines":10,"matches":10,"searches":2,"searches_with_match":2}},"type":"summary"}"#;

    pub const RG_JSON_MATCH_MULTILINE: &str = r#"{"type":"match","data":{"path":{"text":"./foo/baz"},"lines":{"text":"baz 1\n22\n333 bar 4444\n"},"line_number":3,"absolute_offset":16,"submatches":[{"match":{"text":"1\n22\n333"},"start":4,"end":12},{"match":{"text":"4444"},"start":17,"end":21}]}}"#;

    pub const RG_JSON_MATCH_LINE_WRAP: &str = r#"{"type":"match","data":{"path":{"text":"./foo/baz"},"lines":{"text":"123456789!123456789@123456789#123456789$123456789%123456789^123456789&123456789*123456789(123456789_one_hundred_characters_wowzers\n"},"line_number":3,"absolute_offset":16,"submatches":[{"match":{"text":"one_hundred"},"start":100,"end":111}]}}"#;
    pub const RG_JSON_MATCH_LINE_WRAP_MULTI: &str = r#"{"type":"match","data":{"path":{"text":"./foo/baz"},"lines":{"text":"foo foo foo foo foo bar foo foo foo foo foo bar foo foo foo foo foo bar foo foo foo foo foo bar foo foo foo foo foo bar foo foo foo foo foo bar foo foo foo foo foo bar\n"},"line_number":1,"absolute_offset":0,"submatches":[{"match":{"text":"bar"},"start":20,"end":23},{"match":{"text":"bar"},"start":44,"end":47},{"match":{"text":"bar"},"start":68,"end":71},{"match":{"text":"bar"},"start":92,"end":95},{"match":{"text":"bar"},"start":116,"end":119},{"match":{"text":"bar"},"start":140,"end":143},{"match":{"text":"bar"},"start":164,"end":167}]}}"#;
    pub const RG_JSON_CONTEXT_LINE_WRAP: &str = r#"{"type":"context","data":{"path":{"text":"./foo/baz"},"lines":{"text":"123456789!123456789@123456789#123456789$123456789%123456789^123456789&123456789*123456789(123456789_a_context_line\n"},"line_number":4,"absolute_offset":131,"submatches":[]}}"#;

    pub const RG_B64_JSON_BEGIN: &str =
        r#"{"type":"begin","data":{"path":{"bytes":"Li9hL2Zv/28="}}}"#;
    pub const RG_B64_JSON_MATCH: &str = r#"{"type":"match","data":{"path":{"text":"src/model/item.rs"},"lines":{"bytes":"ICAgIP9JdGVtOjr/bmV3KHJnX21zZykK"},"line_number":197,"absolute_offset":5522,"submatches":[{"match":{"text":"Item"},"start":5,"end":9},{"match":{"text":"rg_msg"},"start":16,"end":22}]}}"#;
    pub const RG_B64_JSON_CONTEXT: &str = r#"{"type":"context","data":{"path":{"text":"src/model/item.rs"},"lines":{"bytes":"ICD/fQo="},"line_number":198,"absolute_offset":5544,"submatches":[]}}"#;
    pub const RG_B64_JSON_END: &str = r#"{"type":"end","data":{"path":{"bytes":"Li9hL2Zv/28="},"binary_offset":null,"stats":{"elapsed":{"secs":0,"nanos":64302,"human":"0.000064s"},"searches":1,"searches_with_match":1,"bytes_searched":4,"bytes_printed":235,"matched_lines":1,"matches":1}}}"#;

    impl RgMessage<'_> {
        pub fn from_str(raw_json: &str) -> RgMessage {
            serde_json::from_str::<RgMessage>(raw_json).unwrap()
        }
    }

    impl<'a> ArbitraryData<'a> {
        pub fn new_with_text(text: &'a str) -> ArbitraryData<'a> {
            ArbitraryData::Text { text }
        }

        pub fn new_with_base64(bytes: &'a str) -> ArbitraryData<'a> {
            ArbitraryData::Base64 { bytes }
        }
    }

    impl<'a> SubMatch<'a> {
        pub fn new_text(text: &'a impl AsRef<str>, range: Range<usize>) -> SubMatch<'a> {
            SubMatch {
                text: ArbitraryData::new_with_text(text.as_ref()),
                range,
            }
        }
        pub fn new_base64(b64: &'a  impl AsRef<str>, range: Range<usize>) -> SubMatch<'a> {
            SubMatch {
                text: ArbitraryData::new_with_base64(b64.as_ref()),
                range,
            }
        }
    }
}