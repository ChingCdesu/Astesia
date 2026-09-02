use std::ops::Deref;

use serde_json::Value;

use crate::db::ColumnInfo;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GridColumnKind {
    Boolean,
    Integer,
    Decimal,
    Number,
    Date,
    Time,
    DateTime,
    Enum,
    Json,
    Text,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GridCellInputError {
    NullNotAllowed,
    ExpectedBoolean,
    ExpectedInteger,
    ExpectedNumber,
    ExpectedDate,
    ExpectedTime,
    ExpectedDateTime,
    ExpectedEnum,
    EnumValuesUnavailable,
    InvalidJson,
}

#[derive(Clone, Debug)]
pub(crate) struct GridColumn {
    info: ColumnInfo,
    kind: GridColumnKind,
    enum_values: Vec<String>,
}

impl GridColumn {
    pub(crate) fn new(info: ColumnInfo, enum_values: Vec<String>) -> Self {
        let kind = if enum_values.is_empty() {
            classify_column(&info.data_type)
        } else {
            GridColumnKind::Enum
        };
        Self {
            info,
            kind,
            enum_values,
        }
    }

    pub(crate) const fn kind(&self) -> GridColumnKind {
        self.kind
    }

    pub(crate) fn set_enum_values(&mut self, values: Vec<String>) {
        if !values.is_empty() {
            self.kind = GridColumnKind::Enum;
        }
        self.enum_values = values;
    }

    pub(crate) fn parse_input(
        &self,
        input: &str,
        null_requested: bool,
    ) -> Result<Value, GridCellInputError> {
        if null_requested {
            return if self.nullable {
                Ok(Value::Null)
            } else {
                Err(GridCellInputError::NullNotAllowed)
            };
        }
        let trimmed = input.trim();
        match self.kind {
            GridColumnKind::Boolean => match trimmed.to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(Value::Bool(true)),
                "false" | "0" => Ok(Value::Bool(false)),
                _ => Err(GridCellInputError::ExpectedBoolean),
            },
            GridColumnKind::Integer => serde_json::from_str::<Value>(trimmed)
                .ok()
                .filter(|value| value.as_i64().is_some() || value.as_u64().is_some())
                .ok_or(GridCellInputError::ExpectedInteger),
            GridColumnKind::Decimal => trimmed
                .parse::<sqlx::types::BigDecimal>()
                .map(|_| Value::String(trimmed.to_string()))
                .map_err(|_| GridCellInputError::ExpectedNumber),
            GridColumnKind::Number => serde_json::from_str::<Value>(trimmed)
                .ok()
                .filter(Value::is_number)
                .ok_or(GridCellInputError::ExpectedNumber),
            GridColumnKind::Date => chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                .map(|_| Value::String(trimmed.to_string()))
                .map_err(|_| GridCellInputError::ExpectedDate),
            GridColumnKind::Time => valid_time(trimmed)
                .then(|| Value::String(trimmed.to_string()))
                .ok_or(GridCellInputError::ExpectedTime),
            GridColumnKind::DateTime => valid_datetime(trimmed)
                .then(|| Value::String(trimmed.to_string()))
                .ok_or(GridCellInputError::ExpectedDateTime),
            GridColumnKind::Enum if self.enum_values.is_empty() => {
                Err(GridCellInputError::EnumValuesUnavailable)
            }
            GridColumnKind::Enum => self
                .enum_values
                .iter()
                .any(|value| value == input)
                .then(|| Value::String(input.to_string()))
                .ok_or(GridCellInputError::ExpectedEnum),
            GridColumnKind::Json => serde_json::from_str::<Value>(trimmed)
                .map(|_| Value::String(trimmed.to_string()))
                .map_err(|_| GridCellInputError::InvalidJson),
            GridColumnKind::Text => Ok(Value::String(input.to_string())),
        }
    }
}

impl Deref for GridColumn {
    type Target = ColumnInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

fn classify_column(data_type: &str) -> GridColumnKind {
    let data_type = data_type.trim().to_ascii_lowercase();
    let base = data_type.split(['(', ' ', '[']).next().unwrap_or_default();
    if matches!(
        data_type.as_str(),
        "boolean" | "bool" | "bit" | "tinyint(1)"
    ) {
        GridColumnKind::Boolean
    } else if matches!(data_type.as_str(), "json" | "jsonb") {
        GridColumnKind::Json
    } else if base == "date" {
        GridColumnKind::Date
    } else if matches!(base, "time" | "timetz") {
        GridColumnKind::Time
    } else if matches!(
        base,
        "datetime" | "datetime2" | "smalldatetime" | "timestamp" | "timestamptz"
    ) {
        GridColumnKind::DateTime
    } else if base == "enum" {
        GridColumnKind::Enum
    } else if matches!(
        base,
        "int"
            | "integer"
            | "bigint"
            | "smallint"
            | "tinyint"
            | "mediumint"
            | "serial"
            | "bigserial"
    ) {
        GridColumnKind::Integer
    } else if matches!(base, "numeric" | "decimal") {
        GridColumnKind::Decimal
    } else if matches!(base, "float" | "double" | "real") {
        GridColumnKind::Number
    } else {
        GridColumnKind::Text
    }
}

fn valid_time(input: &str) -> bool {
    chrono::NaiveTime::parse_from_str(input, "%H:%M:%S%.f").is_ok()
        || input
            .char_indices()
            .skip(1)
            .find(|(_, character)| matches!(character, '+' | '-'))
            .is_some_and(|(offset, _)| {
                chrono::NaiveTime::parse_from_str(&input[..offset], "%H:%M:%S%.f").is_ok()
                    && input[offset..].parse::<chrono::FixedOffset>().is_ok()
            })
}

fn valid_datetime(input: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(input).is_ok()
        || chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
        || chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S%.f").is_ok()
}
