use crate::protocol::*;
use anyhow::{Result, anyhow};

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}

pub fn parse(line: &String) -> Result<Command> {
    let v: Vec<&str> = line.split(char::is_whitespace).collect();
    match v[0] {
        "get" => Ok(Command::Get(v[1].into())),
        "version" => Ok(Command::Version),
        _ => Err(anyhow!("unknown command: {}", line)),
    }
}
