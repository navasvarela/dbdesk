//! `CREATE SCHEMA` and `DROP SCHEMA`, or `CREATE DATABASE` and `DROP DATABASE` where the
//! schemas in the tree are databases.

use super::{DdlError, SchemaSpec, quote_name, require_name};
use crate::Dialect;
use crate::dialect::SchemaCreation;

pub(super) fn create(spec: &SchemaSpec, dialect: Dialect) -> Result<String, DdlError> {
    require_name(&spec.name, "schema")?;
    let capabilities = dialect.capabilities();
    let name = quote_name(dialect, &spec.name);
    let owner = spec
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|owner| !owner.is_empty());
    if owner.is_some() && !capabilities.schema_owner {
        return Err(DdlError::Unsupported {
            what: "schema owners",
            dialect,
        });
    }
    Ok(match capabilities.schema_creation {
        SchemaCreation::Unsupported => return Err(unsupported(dialect)),
        SchemaCreation::CreateSchema => match owner {
            Some(owner) => format!(
                "CREATE SCHEMA {name} AUTHORIZATION {}",
                quote_name(dialect, owner)
            ),
            None => format!("CREATE SCHEMA {name}"),
        },
        SchemaCreation::CreateDatabase => format!("CREATE DATABASE {name}"),
    })
}

/// `cascade` drops what the schema holds with it, where the dialect can. `DROP DATABASE`
/// always does, so it needs no `CASCADE`.
pub(super) fn drop(schema: &str, cascade: bool, dialect: Dialect) -> Result<String, DdlError> {
    require_name(schema, "schema")?;
    let capabilities = dialect.capabilities();
    let name = quote_name(dialect, schema);
    Ok(match capabilities.schema_creation {
        SchemaCreation::Unsupported => return Err(unsupported(dialect)),
        SchemaCreation::CreateSchema if cascade => {
            if !capabilities.drop_schema_cascade {
                return Err(DdlError::Unsupported {
                    what: "CASCADE for dropping a schema",
                    dialect,
                });
            }
            format!("DROP SCHEMA {name} CASCADE")
        }
        SchemaCreation::CreateSchema => format!("DROP SCHEMA {name}"),
        SchemaCreation::CreateDatabase => format!("DROP DATABASE {name}"),
    })
}

fn unsupported(dialect: Dialect) -> DdlError {
    DdlError::Unsupported {
        what: "schemas DbRover can create or drop",
        dialect,
    }
}
