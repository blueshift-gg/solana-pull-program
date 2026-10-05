//! pull-core for JavaScript. Terms cross the boundary as JSON; bytes and
//! text come from the same code the program runs, so no JavaScript
//! reimplements the codec, the validity rules or the canonical text.
//!
//! Integers that can exceed 2^53 (amounts, the salt) are strings.

use pull_core::render::render;
use pull_core::terms::{Decay, Limit, Per, Receive, Terms};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TermsJson {
    authority: String,
    spender: Option<String>,
    not_before: i64,
    not_after: Option<i64>,
    salt: String,
    limit: LimitJson,
    receive: Option<ReceiveJson>,
}

#[derive(Serialize, Deserialize)]
struct LimitJson {
    from: String,
    mint: String,
    max: String,
    per: PerJson,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PerJson {
    Total,
    Every(u32),
}

#[derive(Serialize, Deserialize)]
struct ReceiveJson {
    to: String,
    mint: String,
    min: String,
    decay: Option<DecayJson>,
}

#[derive(Serialize, Deserialize)]
struct DecayJson {
    t0: i64,
    t1: i64,
    min: String,
}

/// Canonical bytes for the terms, after the validity rules (`Terms::validate`).
#[wasm_bindgen(js_name = encodeTerms)]
pub fn encode_terms(json: &str) -> Result<Vec<u8>, JsError> {
    let j: TermsJson = serde_json::from_str(json)?;
    let authority = key(&j.authority)?;
    let spender = j.spender.as_deref().map(key).transpose()?;

    // Own every key first; the core's terms borrow them.
    let (from, mint) = (key(&j.limit.from)?, key(&j.limit.mint)?);
    let limit = Limit {
        from: &from,
        mint: &mint,
        max: j.limit.max.parse()?,
        per: match j.limit.per {
            PerJson::Total => Per::Total,
            PerJson::Every(seconds) => Per::Every(seconds),
        },
    };
    let paid = match &j.receive {
        Some(x) => Some((key(&x.to)?, key(&x.mint)?)),
        None => None,
    };
    let receive = match (&j.receive, &paid) {
        (Some(x), Some((to, mint))) => Some(Receive {
            to,
            mint,
            min: x.min.parse()?,
            decay: match &x.decay {
                Some(d) => Some(Decay {
                    t0: d.t0,
                    t1: d.t1,
                    min: d.min.parse()?,
                }),
                None => None,
            },
        }),
        _ => None,
    };

    let terms = Terms {
        cluster: pull_core::constants::CLUSTER,
        authority: &authority,
        spender: spender.as_ref(),
        not_before: j.not_before,
        not_after: j.not_after,
        salt: j.salt.parse()?,
        limit,
        receive,
    };
    terms.validate().map_err(error)?;
    let mut bytes = Vec::new();
    terms.write(&mut bytes);
    Ok(bytes)
}

/// The terms behind canonical bytes, as JSON. Fails on anything non-canonical.
#[wasm_bindgen(js_name = decodeTerms)]
pub fn decode_terms(bytes: &[u8]) -> Result<String, JsError> {
    let t = Terms::decode(bytes).map_err(error)?;
    let json = TermsJson {
        authority: b58(t.authority),
        spender: t.spender.map(b58),
        not_before: t.not_before,
        not_after: t.not_after,
        salt: t.salt.to_string(),
        limit: LimitJson {
            from: b58(t.limit.from),
            mint: b58(t.limit.mint),
            max: t.limit.max.to_string(),
            per: match t.limit.per {
                Per::Total => PerJson::Total,
                Per::Every(seconds) => PerJson::Every(seconds),
            },
        },
        receive: t.receive.map(|x| ReceiveJson {
            to: b58(x.to),
            mint: b58(x.mint),
            min: x.min.to_string(),
            decay: x.decay.map(|d| DecayJson {
                t0: d.t0,
                t1: d.t1,
                min: d.min.to_string(),
            }),
        }),
    };
    Ok(serde_json::to_string(&json)?)
}

/// The canonical text a wallet shows and signs. `decimals` maps each mint to its decimals.
#[wasm_bindgen(js_name = renderText)]
pub fn render_text(bytes: &[u8], decimals: &str) -> Result<String, JsError> {
    let decimals: HashMap<String, u8> = serde_json::from_str(decimals)?;
    let terms = Terms::decode(bytes).map_err(error)?;
    let mut out = Vec::new();
    render(
        &terms,
        |mint| {
            decimals
                .get(&b58(mint))
                .copied()
                .ok_or(pull_core::errors::PullError::InvalidTarget)
        },
        &mut out,
    )
    .map_err(error)?;
    Ok(String::from_utf8(out).expect("canonical text is ASCII"))
}

/// The name of a Pull program error code, as a client sees it in a failed transaction.
#[wasm_bindgen(js_name = errorName)]
pub fn error_name(code: u32) -> Option<String> {
    pull_core::errors::PullError::ALL
        .get(code as usize)
        .map(|e| format!("{e:?}"))
}

fn key(s: &str) -> Result<[u8; 32], JsError> {
    let mut out = [0; 32];
    five8::decode_32(s, &mut out).map_err(|_| JsError::new(&format!("not an address: {s}")))?;
    Ok(out)
}

fn b58(key: &[u8; 32]) -> String {
    let mut out = [0; five8::BASE58_ENCODED_32_MAX_LEN];
    let len = five8::encode_32(key, &mut out) as usize;
    String::from_utf8_lossy(&out[..len]).into_owned()
}

fn error(e: pull_core::errors::PullError) -> JsError {
    JsError::new(&format!("{e:?}"))
}
