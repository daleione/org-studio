use sqlx::{Decode, mysql::MySqlValueRef};

pub(super) fn cell(value: MySqlValueRef<'_>, kind: &str, binary: bool) -> Result<String, String> {
    if binary {
        let name = kind.split_whitespace().next().unwrap_or("");
        let decoded = match name {
            "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" => {
                if kind.contains("UNSIGNED") {
                    <u64 as Decode<sqlx::MySql>>::decode(value).map(|n| n.to_string())
                } else {
                    <i64 as Decode<sqlx::MySql>>::decode(value).map(|n| n.to_string())
                }
            }
            "FLOAT" => <f32 as Decode<sqlx::MySql>>::decode(value).map(|n| n.to_string()),
            "DOUBLE" => <f64 as Decode<sqlx::MySql>>::decode(value).map(|n| n.to_string()),
            "TIME" => <sqlx::mysql::types::MySqlTime as Decode<sqlx::MySql>>::decode(value)
                .map(|n| n.to_string()),
            "DATE" | "DATETIME" | "TIMESTAMP" => {
                let bytes = <&[u8] as Decode<sqlx::MySql>>::decode(value)
                    .map_err(|_| "Could not decode MySQL date")?;
                return date(bytes, name != "DATE");
            }
            _ => return text(value),
        };
        return decoded.map_err(|_| "Could not decode a MySQL numeric or time value".into());
    }
    text(value)
}

fn text(value: MySqlValueRef<'_>) -> Result<String, String> {
    let bytes = <&[u8] as Decode<sqlx::MySql>>::decode(value)
        .map_err(|_| "Could not decode a MySQL cell")?;
    Ok(match std::str::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => text.to_owned(),
        _ => {
            use std::fmt::Write;
            let mut text = String::from("0x");
            for byte in bytes {
                let _ = write!(text, "{byte:02X}");
            }
            text
        }
    })
}

fn date(bytes: &[u8], time: bool) -> Result<String, String> {
    let len = bytes.first().copied().unwrap_or(0) as usize;
    if bytes.len() != len + 1 || ![0, 4, 7, 11].contains(&len) {
        return Err("Invalid MySQL binary date".into());
    }
    let mut text = if len == 0 {
        "0000-00-00".into()
    } else {
        format!(
            "{:04}-{:02}-{:02}",
            u16::from_le_bytes([bytes[1], bytes[2]]),
            bytes[3],
            bytes[4]
        )
    };
    if time {
        if len >= 7 {
            text.push_str(&format!(" {:02}:{:02}:{:02}", bytes[5], bytes[6], bytes[7]));
        } else {
            text.push_str(" 00:00:00");
        }
        if len == 11 {
            text.push_str(&format!(
                ".{:06}",
                u32::from_le_bytes(bytes[8..12].try_into().unwrap())
            ));
        }
    }
    Ok(text)
}
