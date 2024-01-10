use json::{parse_json, JsonValue, ParseResult};
use std::{collections::HashMap, io};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task;

#[tokio::main]
async fn main() -> io::Result<()> {
    let server_sock = TcpListener::bind("0.0.0.0:10000").await?;
    loop {
        if let Ok((sock, _)) = server_sock.accept().await {
            task::spawn(async move {
                let (read_half, mut write_half) = sock.into_split();
                let mut lines = BufReader::new(read_half).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    println!(" IN {}", line);
                    let (valid, resp) = process_line(line);
                    write_half.write(&resp.as_bytes()).await?;
                    println!("OUT {}", resp);
                    if !valid {
                        break;
                    }
                }

                io::Result::<()>::Ok(())
            });
        }
    }
}

fn process_line(line: String) -> (bool, String) {
    let parse_result = parse_json(&line);
    match extract_number(&parse_result) {
        None => (false, String::from("{\n")),
        Some(num) => {
            let prime = is_prime(num);

            let json_resp = JsonValue::Object(HashMap::from([
                (
                    "method".to_string(),
                    JsonValue::String("isPrime".to_string()),
                ),
                ("prime".to_string(), JsonValue::Boolean(prime)),
            ]));
            (true, json_resp.to_string() + "\n")
        }
    }
}

fn is_prime(float: f64) -> bool {
    let num = if float.fract() < 1e-10 {
        float.round() as i64
    } else {
        return false;
    };

    if num < 2 {
        return false;
    }
    if num == 2 {
        return true;
    }

    let upper_bound = (num as f64).sqrt().ceil() as i64;

    (2..=upper_bound)
        .into_iter()
        .find(|&i| num % i == 0)
        .is_none()
}

fn extract_number(json_result: &ParseResult) -> Option<f64> {
    match json_result {
        Err(_) => None,
        Ok(json) => {
            if let JsonValue::Object(obj) = json {
                if obj.get("method") != Some(&JsonValue::String("isPrime".to_string())) {
                    return None;
                }
                match obj.get("number") {
                    None => None,
                    Some(JsonValue::Number(float)) => {
                        Some(*float)
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prime() {
        let test_values = [
            (0.0, false),
            (1.0, false),
            (-1.0, false),
            (-1337.0, false),
            (2.0, true),
            (3.0, true),
            (7.0, true),
            (61.0, true),
            (61.123, false),
        ];

        for (num, expected_result) in test_values {
            assert_eq!(is_prime(num), expected_result);
        }
    }
}
