"""Shared PostgreSQL observer configuration for SETUP proof and live arms."""

ROLE = "parity_subject"
DATABASE = "mesh"
PREFIX = "dense-plan|%n|%p|%l|%u|%d| "
SAMPLE_RATE = "0.35"


def role_sql(module, sample_rate=SAMPLE_RATE):
    settings = {
        "session_preload_libraries": str(module),
        "pg_stat_statements.track": "all",
        "auto_explain.log_min_duration": "0",
        "auto_explain.log_analyze": "on",
        "auto_explain.log_buffers": "on",
        "auto_explain.log_wal": "on",
        "auto_explain.log_timing": "off",
        "auto_explain.log_verbose": "on",
        "auto_explain.log_nested_statements": "on",
        "auto_explain.log_parameter_max_length": "0",
        "auto_explain.log_format": "json",
        "auto_explain.sample_rate": sample_rate,
    }
    return "\n".join(
        f"ALTER ROLE {ROLE} IN DATABASE {DATABASE} SET {key} = {sql_literal(value)};"
        for key, value in settings.items()
    )


def sql_literal(value):
    return "'" + value.replace("'", "''") + "'"
