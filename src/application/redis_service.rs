use crate::{
    connection_runtime::DriverHandle,
    db::{DbType, QueryResult},
};

use super::{connections::ConnectionManager, QueryTarget};

pub(crate) use crate::db::{
    RedisKeySnapshot, RedisListSide, RedisMutation, RedisPageCursor, RedisValue,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RedisCommand {
    arguments: Vec<String>,
}

impl RedisCommand {
    pub(crate) fn parse(input: &str) -> Result<Self, String> {
        let arguments = parse_redis_arguments(input)?;
        if arguments.is_empty() {
            return Err("Redis command is empty".to_string());
        }
        Ok(Self { arguments })
    }
}

#[derive(Clone)]
pub(crate) struct RedisService {
    manager: ConnectionManager,
}

impl RedisService {
    pub(super) fn new(manager: ConnectionManager) -> Self {
        Self { manager }
    }

    pub(crate) async fn scan_keys(
        &self,
        target: &QueryTarget,
        contains: &str,
    ) -> Result<Vec<String>, String> {
        let handle = self.driver(target).await?;
        let driver = handle.lock_active().await?;
        driver
            .scan_redis_keys(&target.database, &contains_pattern(contains))
            .await
            .map_err(|error| format!("Could not scan Redis keys: {error}"))
    }

    pub(crate) async fn key(
        &self,
        target: &QueryTarget,
        key: &str,
    ) -> Result<RedisKeySnapshot, String> {
        self.key_page(target, key, None).await
    }

    pub(crate) async fn key_page(
        &self,
        target: &QueryTarget,
        key: &str,
        cursor: Option<RedisPageCursor>,
    ) -> Result<RedisKeySnapshot, String> {
        let handle = self.driver(target).await?;
        let driver = handle.lock_active().await?;
        let result = match cursor {
            None => driver.get_redis_key(&target.database, key).await,
            Some(_) => {
                driver
                    .get_redis_key_page(&target.database, key, cursor)
                    .await
            }
        };
        result.map_err(|error| format!("Could not load Redis key: {error}"))
    }

    pub(crate) async fn mutate(
        &self,
        target: &QueryTarget,
        key: &str,
        mutation: RedisMutation,
    ) -> Result<u64, String> {
        let handle = self.driver(target).await?;
        let driver = handle.lock_active().await?;
        driver
            .mutate_redis_key(&target.database, key, mutation)
            .await
            .map_err(|error| format!("Could not update Redis key: {error}"))
    }

    pub(crate) async fn execute(
        &self,
        target: &QueryTarget,
        command: RedisCommand,
    ) -> Result<QueryResult, String> {
        let handle = self.driver(target).await?;
        let driver = handle.lock_active().await?;
        driver
            .execute_redis_command(&target.database, command.arguments)
            .await
            .map_err(|error| format!("Redis command failed: {error}"))
    }

    async fn driver(&self, target: &QueryTarget) -> Result<DriverHandle, String> {
        if target.db_type != DbType::Redis {
            return Err("Redis operations require a Redis target".to_string());
        }
        let (handle, generation) = self.manager.driver_session(&target.connection_id).await?;
        if generation != target.session_generation {
            return Err(format!(
                "Redis session changed (expected {}, found {generation})",
                target.session_generation
            ));
        }
        Ok(handle)
    }
}

fn contains_pattern(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() {
        return "*".to_string();
    }
    let mut pattern = String::with_capacity(input.len() + 2);
    pattern.push('*');
    for character in input.chars() {
        if matches!(character, '*' | '?' | '[' | ']' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('*');
    pattern
}

fn parse_redis_arguments(input: &str) -> Result<Vec<String>, String> {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut argument_started = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            argument_started = true;
            escaped = false;
            continue;
        }
        if character == '\\' {
            argument_started = true;
            escaped = true;
            continue;
        }
        match (quote, character) {
            (Quote::None, '\'') => {
                quote = Quote::Single;
                argument_started = true;
            }
            (Quote::None, '"') => {
                quote = Quote::Double;
                argument_started = true;
            }
            (Quote::Single, '\'') | (Quote::Double, '"') => quote = Quote::None,
            (Quote::None, character) if character.is_whitespace() => {
                if argument_started {
                    arguments.push(std::mem::take(&mut current));
                    argument_started = false;
                }
            }
            (_, character) => {
                current.push(character);
                argument_started = true;
            }
        }
    }
    if escaped {
        return Err("Redis command ends with an incomplete escape".to_string());
    }
    if quote != Quote::None {
        return Err("Redis command contains an unterminated quote".to_string());
    }
    if argument_started {
        arguments.push(current);
    }
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_commands_preserve_quoted_and_escaped_arguments() {
        assert_eq!(
            RedisCommand::parse("HSET 'key with spaces' field \"value with spaces\"")
                .unwrap()
                .arguments,
            ["HSET", "key with spaces", "field", "value with spaces"]
        );
        assert_eq!(
            RedisCommand::parse("SET key\\ with\\ spaces value")
                .unwrap()
                .arguments,
            ["SET", "key with spaces", "value"]
        );
        assert_eq!(
            RedisCommand::parse("SET key \"\"").unwrap().arguments,
            ["SET", "key", ""]
        );
    }

    #[test]
    fn key_search_escapes_redis_glob_metacharacters() {
        assert_eq!(contains_pattern(""), "*");
        assert_eq!(contains_pattern("invoice[1]*"), "*invoice\\[1\\]\\**");
    }

    #[test]
    fn raw_commands_reject_incomplete_syntax() {
        assert!(RedisCommand::parse("SET key \\").is_err());
        assert!(RedisCommand::parse("SET key 'value").is_err());
    }
}
