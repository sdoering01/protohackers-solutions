use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
    result::Result,
};

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Object(HashMap<String, JsonValue>),
    Array(Vec<JsonValue>),
    String(String),
    Number(f64),
    Boolean(bool),
    Null,
}

impl Display for JsonValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            JsonValue::Object(obj) => {
                f.write_str("{")?;
                for (idx, (key, val)) in obj.iter().enumerate() {
                    f.write_fmt(format_args!(r#""{}":{}"#, key, val))?;
                    if idx < obj.len() - 1 {
                        f.write_str(",")?;
                    }
                }
                f.write_str("}")?;
            }
            JsonValue::Array(arr) => {
                f.write_str("[")?;
                for (idx, el) in arr.iter().enumerate() {
                    f.write_str(&el.to_string())?;
                    if idx < arr.len() - 1 {
                        f.write_str(",")?;
                    }
                }
                f.write_str("]")?;
            }
            JsonValue::String(string) => f.write_fmt(format_args!(r#""{}""#, string))?,
            JsonValue::Number(num) => f.write_fmt(format_args!("{}", num))?,
            JsonValue::Boolean(b) => f.write_str(if *b { "true" } else { "false" })?,
            JsonValue::Null => f.write_str("null")?,
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    UnexpectedJsonEnd,
    // TODO: Add position where the error occured
    UnexpectedSymbol(char),
    UnexpectedStringEnd,
    UnexpectedNumberEnd,
}

pub type ParseResult = Result<JsonValue, ParseError>;

pub fn parse_json(mut string: &str) -> ParseResult {
    let v = parse_json_value(&mut string)?;
    match string.chars().next() {
        None => Ok(v),
        Some(c) => Err(ParseError::UnexpectedSymbol(c)),
    }
}

fn parse_json_value(string: &mut &str) -> ParseResult {
    trim_whitespaces(string);

    // Checked for length 0 above
    let ret = match string.chars().next() {
        Some('"') => parse_json_string(string)?,
        Some('-') | Some('0'..='9') => parse_json_number(string)?,
        Some('{') => parse_json_object(string)?,
        Some('[') => parse_json_array(string)?,
        Some('t') if string.starts_with("true") => {
            advance_by(string, 4);
            JsonValue::Boolean(true)
        }
        Some('f') if string.starts_with("false") => {
            advance_by(string, 5);
            JsonValue::Boolean(false)
        }
        Some('n') if string.starts_with("null") => {
            advance_by(string, 4);
            JsonValue::Null
        }
        Some(c) => return Err(ParseError::UnexpectedSymbol(c)),
        None => return Err(ParseError::UnexpectedJsonEnd),
    };

    trim_whitespaces(string);
    Ok(ret)
}

fn trim_whitespaces(string: &mut &str) {
    *string = string.trim_start_matches(char::is_whitespace);
}

fn advance_by(string: &mut &str, by: usize) {
    *string = &string[by..];
}

/// `string` has to start with "
fn parse_json_string(string: &mut &str) -> ParseResult {
    match string.chars().next() {
        None => return Err(ParseError::UnexpectedJsonEnd),
        Some('"') => {}
        Some(c) => return Err(ParseError::UnexpectedSymbol(c)),
    };

    // Strip "
    advance_by(string, 1);

    let mut container = String::new();
    let mut chars = string.chars();
    let mut read_bytes = 0;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                advance_by(string, read_bytes + 1);
                return Ok(JsonValue::String(container));
            }
            '\\' => {
                match chars.next() {
                    None => return Err(ParseError::UnexpectedStringEnd),
                    Some(c @ ('"' | '\\' | '/' | '\n' | '\r' | '\t')) => {
                        read_bytes += 2;
                        container.push(c)
                    }
                    Some(_) => {
                        // TODO: Handle other escape characters (backspace, formfeed, unicode)
                        todo!();
                    }
                };
            }
            _ => {
                read_bytes += c.len_utf8();
                container.push(c);
            }
        }
    }

    Err(ParseError::UnexpectedStringEnd)
}

fn parse_json_number(string: &mut &str) -> ParseResult {
    let sign = if string.starts_with('-') {
        advance_by(string, 1);
        -1.0
    } else {
        1.0
    };

    let mut num: f64;
    let mut chars = string.chars().peekable();
    let mut read_chars = 0;
    match chars.next().map(|c| c.to_digit(10)) {
        None => return Err(ParseError::UnexpectedNumberEnd),
        Some(Some(0)) => {
            read_chars += 1;
            num = 0.0;
        }
        Some(Some(digit)) => {
            read_chars += 1;
            num = digit as f64;
            while let Some(c) = chars.peek().copied() {
                match c.to_digit(10) {
                    Some(digit) => {
                        chars.next();
                        read_chars += 1;
                        num *= 10.0;
                        num += digit as f64;
                    }
                    None => break,
                }
            }
        }
        Some(None) => panic!("error trying to parse JSON number: given string is not numeric"),
    }

    // fraction
    if chars.peek() == Some(&'.') {
        chars.next();
        read_chars += 1;

        // Assert that next char is a valid number
        match chars.peek() {
            None => return Err(ParseError::UnexpectedJsonEnd),
            Some('0'..='9') => {}
            Some(c) => return Err(ParseError::UnexpectedSymbol(*c)),
        }

        let mut factor = 1.0;
        while let Some(c) = chars.peek().copied() {
            match c.to_digit(10) {
                Some(digit) => {
                    chars.next();
                    read_chars += 1;
                    factor *= 0.1;
                    num += digit as f64 * factor;
                }
                None => break,
            }
        }
    }

    // TODO: Handle exponent

    advance_by(string, read_chars);

    Ok(JsonValue::Number(sign * num))
}

fn parse_json_object(string: &mut &str) -> ParseResult {
    let mut obj = HashMap::new();

    // Strip {
    advance_by(string, 1);
    trim_whitespaces(string);

    let mut first_key = true;
    loop {
        match string.chars().next() {
            None => return Err(ParseError::UnexpectedJsonEnd),
            Some('"') if first_key => {
                first_key = false;
                parse_json_key_value_pair(string, &mut obj)?;
            }
            Some(',') if !first_key => {
                advance_by(string, 1);
                trim_whitespaces(string);
                parse_json_key_value_pair(string, &mut obj)?;
            }
            Some('}') => {
                advance_by(string, 1);
                break;
            }
            Some(c) => return Err(ParseError::UnexpectedSymbol(c)),
        }
    }

    Ok(JsonValue::Object(obj))
}

fn parse_json_key_value_pair(
    string: &mut &str,
    obj: &mut HashMap<String, JsonValue>,
) -> Result<(), ParseError> {
    let key = match parse_json_string(string)? {
        JsonValue::String(k) => k,
        _ => unreachable!(),
    };

    trim_whitespaces(string);
    match string.chars().next() {
        None => return Err(ParseError::UnexpectedJsonEnd),
        Some(':') => {
            advance_by(string, 1);
            let val = parse_json_value(string)?;
            obj.insert(key, val);
        }
        Some(c) => return Err(ParseError::UnexpectedSymbol(c)),
    };
    Ok(())
}

fn parse_json_array(string: &mut &str) -> ParseResult {
    let mut arr = Vec::new();

    advance_by(string, 1);
    trim_whitespaces(string);

    let mut first_element = true;
    loop {
        match string.chars().next() {
            None => return Err(ParseError::UnexpectedJsonEnd),
            Some(']') => {
                advance_by(string, 1);
                break;
            }
            _ if first_element => {
                first_element = false;
                let val = parse_json_value(string)?;
                arr.push(val);
            }
            Some(',') if !first_element => {
                advance_by(string, 1);
                let val = parse_json_value(string)?;
                arr.push(val);
            }
            Some(c) => return Err(ParseError::UnexpectedSymbol(c)),
        }
    }

    Ok(JsonValue::Array(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_parsing() {
        let test_values = [
            (r#""abc""#, Ok(JsonValue::String("abc".to_string()))),
            (r#"""#, Err(ParseError::UnexpectedStringEnd)),
            (r#"  "abc  "#, Err(ParseError::UnexpectedStringEnd)),
            (r#""""#, Ok(JsonValue::String(String::new()))),
            (r#"  "abc"   "#, Ok(JsonValue::String("abc".to_string()))),
            (r#"  "abc"  a"#, Err(ParseError::UnexpectedSymbol('a'))),
            (r#""\"""#, Ok(JsonValue::String("\"".to_string()))),
        ];

        for (input, expected_output) in test_values {
            assert_eq!(parse_json(input), expected_output);
        }
    }

    #[test]
    fn test_number_parsing() {
        let test_values = [
            (r#"0"#, Ok(JsonValue::Number(0.0))),
            (r#"0.0"#, Ok(JsonValue::Number(0.0))),
            (r#"0.00"#, Ok(JsonValue::Number(0.0))),
            (r#"00"#, Err(ParseError::UnexpectedSymbol('0'))),
            (r#"123.321"#, Ok(JsonValue::Number(123.321))),
            (r#"20"#, Ok(JsonValue::Number(20.0))),
            (r#"-0"#, Ok(JsonValue::Number(0.0))),
            (r#"-10.5"#, Ok(JsonValue::Number(-10.5))),
            (r#"-10."#, Err(ParseError::UnexpectedJsonEnd)),
            (r#"-10.a"#, Err(ParseError::UnexpectedSymbol('a'))),
        ];

        for (input, expected_output) in test_values {
            assert_eq!(parse_json(input), expected_output);
        }
    }

    #[test]
    fn test_object_parsing() {
        let test_values = [
            (r#"{}"#, Ok(JsonValue::Object(HashMap::new()))),
            (
                r#"{"a":"b","c":true}"#,
                Ok(JsonValue::Object(HashMap::from([
                    ("a".to_string(), JsonValue::String("b".to_string())),
                    ("c".to_string(), JsonValue::Boolean(true)),
                ]))),
            ),
            (
                r#"{"a":"b","c":true,}"#,
                Err(ParseError::UnexpectedSymbol('}')),
            ),
            (
                r#"{"nested":{"one_more_level":{"empty":{}}}}"#,
                Ok(JsonValue::Object(HashMap::from([(
                    "nested".to_string(),
                    JsonValue::Object(HashMap::from([(
                        "one_more_level".to_string(),
                        JsonValue::Object(HashMap::from([(
                            "empty".to_string(),
                            JsonValue::Object(HashMap::new()),
                        )])),
                    )])),
                )]))),
            ),
        ];

        for (input, expected_output) in test_values {
            assert_eq!(parse_json(input), expected_output);
        }
    }

    #[test]
    fn test_array_parsing() {
        let test_values = [
            (r#"[]"#, Ok(JsonValue::Array(vec![]))),
            (
                r#"["abc", 1.1 , false, {}]"#,
                Ok(JsonValue::Array(vec![
                    JsonValue::String("abc".to_string()),
                    JsonValue::Number(1.1),
                    JsonValue::Boolean(false),
                    JsonValue::Object(HashMap::new()),
                ])),
            ),
            (
                r#"[[[]]]"#,
                Ok(JsonValue::Array(vec![JsonValue::Array(vec![
                    JsonValue::Array(vec![]),
                ])])),
            ),
            (r#"[false,true,]"#, Err(ParseError::UnexpectedSymbol(']'))),
        ];

        for (input, expected_output) in test_values {
            assert_eq!(parse_json(input), expected_output);
        }
    }

    #[test]
    fn test_to_string() {
        let test_values = [
            (
                JsonValue::Array(vec![
                    JsonValue::String("abc".to_string()),
                    JsonValue::Number(1.1),
                    JsonValue::Boolean(false),
                    JsonValue::Object(HashMap::new()),
                ]),
                r#"["abc",1.1,false,{}]"#,
            ),
            (
                JsonValue::Array(vec![JsonValue::Array(vec![JsonValue::Array(vec![])])]),
                r#"[[[]]]"#,
            ),
        ];

        for (input, expected_output) in test_values {
            assert_eq!(input.to_string(), expected_output);
        }
    }
}
