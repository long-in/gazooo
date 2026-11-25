//! 本文に書かれてしまった擬似ツール呼び出し(`<suggest_params>…</suggest_params>` 等)の救済。
//!
//! 一部のモデルは、ツールを呼ぶ代わりに XML 風のタグを本文へ出力する。そのままだと提案カードが
//! 出ず、タグも画面に残る。ストリームの本文からタグを取り除き、中身をツール入力として取り出す。

use serde_json::{Map, Value};

use super::context::{TOOL_MODEL_NAME, TOOL_NAME};

const NAMES: [&str; 2] = [TOOL_NAME, TOOL_MODEL_NAME];
const NUMERIC: [&str; 6] = ["width", "height", "steps", "cfg", "guidance", "batchSize"];

/// 本文を流しながら擬似ツール呼び出しを取り除く。タグが途中で切れて届く場合に備えて保留する。
#[derive(Default)]
pub struct TextToolFilter {
    buf: String,
    /// 開いたタグの名前(閉じタグを待っている間)
    open: Option<&'static str>,
}

impl TextToolFilter {
    /// 表示してよい本文と、完成した呼び出し(名前, 入力)を返す。
    pub fn feed(&mut self, text: &str) -> (String, Vec<(String, Value)>) {
        self.buf.push_str(text);
        let mut out = String::new();
        let mut calls = Vec::new();
        loop {
            if let Some(name) = self.open {
                let close = format!("</{name}>");
                let Some(end) = self.buf.find(&close) else {
                    break;
                };
                let body: String = self.buf[..end].to_string();
                self.buf.drain(..end + close.len());
                self.open = None;
                if let Some(v) = parse_body(name, &body) {
                    calls.push((name.to_string(), v));
                }
                continue;
            }
            let Some(lt) = self.buf.find('<') else {
                out.push_str(&self.buf);
                self.buf.clear();
                break;
            };
            out.push_str(&self.buf[..lt]);
            self.buf.drain(..lt);
            if let Some(name) = NAMES
                .iter()
                .find(|n| self.buf.starts_with(&format!("<{n}>")))
            {
                self.buf.drain(..name.len() + 2);
                self.open = Some(name);
                continue;
            }
            // タグの先頭になり得る間は保留、違うと分かったら '<' を本文として出す
            if NAMES
                .iter()
                .any(|n| format!("<{n}>").starts_with(&self.buf))
            {
                break;
            }
            out.push('<');
            self.buf.drain(..1);
        }
        (out, calls)
    }

    /// ストリーム終了時。閉じられなかったタグの中身は捨て、保留中の本文だけを返す。
    pub fn finish(&mut self) -> String {
        let rest = if self.open.is_some() {
            String::new()
        } else {
            std::mem::take(&mut self.buf)
        };
        self.buf.clear();
        self.open = None;
        rest
    }
}

fn decode(s: &str) -> String {
    s.trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

/// `<a>x</a><b>y</b>` の並びを、タグ名 → 値の組にする(入れ子は再帰)。
fn children(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(s) = rest.find('<') {
        let after = &rest[s + 1..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        if tag.is_empty() || tag.starts_with('/') || !tag.chars().all(|c| c.is_ascii_alphanumeric())
        {
            rest = &after[gt + 1..];
            continue;
        }
        let inner_start = &after[gt + 1..];
        let close = format!("</{tag}>");
        let Some(e) = inner_start.find(&close) else {
            break;
        };
        out.push((tag.to_string(), inner_start[..e].to_string()));
        rest = &inner_start[e + close.len()..];
    }
    out
}

fn scalar(key: &str, v: &str) -> Value {
    let t = decode(v);
    if NUMERIC.contains(&key)
        && let Ok(n) = t.parse::<f64>()
    {
        // 整数項目は 30.0 のような値でも検証側(int_in)が受け付ける
        return serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number);
    }
    match t.as_str() {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::String(t),
    }
}

fn object(body: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in children(body) {
        let val = if k == "faceDetailer" {
            let mut f = Map::new();
            for (fk, fv) in children(&v) {
                let t = decode(&fv);
                let val = match fk.as_str() {
                    "enabled" => Value::Bool(t == "true"),
                    "denoise" => t.parse::<f64>().map_or(Value::Null, Value::from),
                    _ => continue,
                };
                f.insert(fk, val);
            }
            Value::Object(f)
        } else {
            scalar(&k, &v)
        };
        m.insert(k, val);
    }
    m
}

fn parse_body(name: &str, body: &str) -> Option<Value> {
    let trimmed = body.trim();
    // JSON をそのまま書くモデルもある
    if trimmed.starts_with('{')
        && let Ok(v) = serde_json::from_str::<Value>(trimmed)
    {
        return Some(v);
    }
    let mut m = Map::new();
    if name == TOOL_NAME {
        for (k, v) in children(body) {
            if k == "changes" {
                m.insert(k, Value::Object(object(&v)));
            } else {
                m.insert(k, Value::String(decode(&v)));
            }
        }
    } else {
        m = object(body);
    }
    (!m.is_empty()).then_some(Value::Object(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&str]) -> (String, Vec<(String, Value)>) {
        let mut f = TextToolFilter::default();
        let (mut t, mut c) = (String::new(), Vec::new());
        for ch in chunks {
            let (a, b) = f.feed(ch);
            t.push_str(&a);
            c.extend(b);
        }
        t.push_str(&f.finish());
        (t, c)
    }

    #[test]
    fn strips_params_tag_split_across_chunks() {
        let (text, calls) = run(&[
            "説明です。\n<sugg",
            "est_params>\n<summary>要約</summary><changes><pro",
            "mpt>a &amp; b</prompt><steps>30</steps></changes></suggest_params>",
        ]);
        assert_eq!(text, "説明です。\n");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "suggest_params");
        assert_eq!(calls[0].1["summary"], "要約");
        assert_eq!(calls[0].1["changes"]["prompt"], "a & b");
        assert_eq!(calls[0].1["changes"]["steps"].as_f64(), Some(30.0));
    }

    #[test]
    fn model_tag_and_plain_angle_brackets() {
        let (text, calls) = run(&[
            "1 < 2 で <b> です <suggest_model><familyId>checkpoint-sdxl</familyId><reason>理由</reason></suggest_model>",
        ]);
        assert_eq!(text, "1 < 2 で <b> です ");
        assert_eq!(calls[0].1["familyId"], "checkpoint-sdxl");
    }

    #[test]
    fn unclosed_tag_is_dropped() {
        let (text, calls) = run(&["本文<suggest_params><summary>x"]);
        assert_eq!(text, "本文");
        assert!(calls.is_empty());
    }
}
