use super::*;
use percent_encoding::percent_decode_str;
use std::collections::BTreeMap;

pub(crate) const CONNECTION_HELP: &str = "Set :dbconnection to a mysql://user:password@host:3306/database URL or a connection name defined in Org properties";

pub(super) struct Connection {
    pub(super) options: MySqlConnectOptions,
    pub(super) password: String,
    pub(super) identity: String,
}

pub(super) fn from_headers(headers: &Headers) -> Result<Option<Connection>, String> {
    let mut password = String::new();
    let mut parameters = BTreeMap::new();
    let mut options = if let Some(connection) = headers.get(":dbconnection") {
        let connection = if connection.contains("://") {
            connection
        } else {
            headers.property(connection).ok_or(
                "Unknown MySQL connection name; define it in this heading, a parent heading or file properties",
            )?
        };
        let mut url = url::Url::parse(connection).map_err(
            |_| "Invalid MySQL connection URL; use mysql://user:password@host:3306/database",
        )?;
        if url.scheme() != "mysql" {
            return Err("Only mysql:// connection URLs are supported".into());
        }
        if url.host_str().is_none() || url.fragment().is_some() || url.port() == Some(0) {
            return Err("Invalid MySQL connection URL; specify a host and URL-encode special characters in credentials".into());
        }
        if let Some(value) = url.password() {
            password = percent_decode_str(value)
                .decode_utf8()
                .map_err(|_| "MySQL passwords must be valid UTF-8")?
                .into_owned();
        }
        for (key, value) in url.query_pairs() {
            let name = match key.as_ref() {
                "sslmode" => "ssl-mode",
                "sslca" => "ssl-ca",
                "sslcert" => "ssl-cert",
                "sslkey" => "ssl-key",
                "time-zone" => "timezone",
                name => name,
            };
            if ![
                "ssl-mode",
                "ssl-ca",
                "ssl-cert",
                "ssl-key",
                "charset",
                "collation",
                "socket",
                "timezone",
                "statement-cache-capacity",
            ]
            .contains(&name)
            {
                return Err(
                    "Unsupported MySQL connection URL option; use SQLx MySQL connection options"
                        .into(),
                );
            }
            let value = if name == "ssl-mode" {
                value.replace('-', "_")
            } else {
                value.into_owned()
            };
            parameters.insert(name.to_owned(), value);
        }
        url.set_query(None);
        url.query_pairs_mut().extend_pairs(&parameters);
        url.as_str().parse::<MySqlConnectOptions>().map_err(
            |_| "Invalid MySQL connection URL options; check port, TLS mode and percent encoding",
        )?
    } else {
        if headers.get(":dbhost").is_none() || headers.get(":dbuser").is_none() {
            return Ok(None);
        }
        MySqlConnectOptions::new()
    };
    // Standard explicit Babel fields override the same fields in the URL.
    if let Some(host) = headers.get(":dbhost") {
        options = options.host(host);
    }
    if let Some(port) = headers.get(":dbport") {
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or("MySQL :dbport must be an integer from 1 to 65535")?;
        options = options.port(port);
    }
    if let Some(user) = headers.get(":dbuser") {
        options = options.username(user);
    }
    if let Some(database) = headers.get(":database") {
        options = options.database(database);
    }
    if let Some(value) = headers.get(":dbpassword") {
        password = value.to_owned();
        options = options.password(value);
    }
    if [
        options.get_host(),
        options.get_username(),
        options.get_database().unwrap_or(""),
    ]
    .iter()
    .any(|value| value.contains(['\r', '\n', '\0']))
        || options.get_host().is_empty()
        || options.get_username().is_empty()
    {
        return Err("MySQL host, user or database contains an invalid value".into());
    }
    parameters.remove("ssl-mode");
    parameters.remove("statement-cache-capacity");
    let identity = serde_json::to_string(&(
        options.get_host(),
        options.get_port(),
        options.get_username(),
        options.get_database(),
        format!("{:?}", options.get_ssl_mode()),
        &parameters,
    ))
    .map_err(|_| "Could not resolve the MySQL connection target")?;
    let mut options = options
        .pipes_as_concat(false)
        .no_engine_substitution(false)
        .statement_cache_capacity(0)
        .disable_statement_logging();
    if !parameters.contains_key("timezone") {
        options = options.timezone(None);
    }
    Ok(Some(Connection {
        options,
        password,
        identity,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::mysql::MySqlSslMode;

    fn connection(url: &str) -> Connection {
        let mut headers = Headers::default();
        headers.parse(&format!(":dbconnection \"{url}\"")).unwrap();
        from_headers(&headers).unwrap().unwrap()
    }

    #[test]
    fn connection_url_decodes_credentials_database_and_tls_options() {
        let c = connection(
            "mysql://reader%40company:p%40ss%3A%2F%25%E5%AF%86@db.example:3307/reports%20db?ssl-mode=VERIFY_CA&ssl-ca=/tmp/ca.pem",
        );
        assert_eq!(c.options.get_host(), "db.example");
        assert_eq!(c.options.get_port(), 3307);
        assert_eq!(c.options.get_username(), "reader@company");
        assert_eq!(c.options.get_database(), Some("reports db"));
        assert_eq!(c.password, "p@ss:/%密");
        assert!(matches!(c.options.get_ssl_mode(), MySqlSslMode::VerifyCa));
        assert!(!c.identity.contains("p@ss"));
        assert!(!c.identity.contains("p%40ss"));
    }

    #[test]
    fn connections_without_password_prefer_tls_by_default() {
        for args in [
            ":dbconnection mysql://reader@localhost/analytics",
            ":dbhost localhost :dbuser reader :database analytics",
        ] {
            let mut headers = Headers::default();
            headers.parse(args).unwrap();
            let c = from_headers(&headers).unwrap().unwrap();
            assert!(c.password.is_empty());
            assert!(matches!(c.options.get_ssl_mode(), MySqlSslMode::Preferred));
            assert_eq!(c.options.get_port(), 3306);
        }
    }

    #[test]
    fn explicit_standard_headers_override_url_fields_and_password() {
        let mut headers = Headers::default();
        headers.parse(":dbconnection mysql://old:old-pass@old.example:3307/old :dbhost new.example :dbport 3308 :dbuser reader :database reports :dbpassword \"new-pass\"").unwrap();
        let c = from_headers(&headers).unwrap().unwrap();
        assert_eq!(c.options.get_host(), "new.example");
        assert_eq!(c.options.get_port(), 3308);
        assert_eq!(c.options.get_username(), "reader");
        assert_eq!(c.options.get_database(), Some("reports"));
        assert_eq!(c.password, "new-pass");
    }

    #[test]
    fn invalid_urls_fail_without_disclosing_credentials_or_loading_profiles() {
        for url in [
            "analytics_ro",
            "postgres://reader:secret-password@db.example/db",
            "mysql://reader:secret-password@db.example:bad/db",
            "mysql://reader:secret-password@db.example/db?ssl-mode=secret-password",
            "mysql://reader:secret-password@db.example/db?unknown=value",
            "mysql://reader:secret-password@db.example/db#fragment",
        ] {
            let mut headers = Headers::default();
            headers.parse(&format!(":dbconnection \"{url}\"")).unwrap();
            let error = from_headers(&headers)
                .err()
                .expect("invalid connection must fail");
            assert!(!error.contains("secret-password"), "{error}");
        }
        assert!(from_headers(&Headers::default()).unwrap().is_none());
    }

    #[test]
    fn trust_identity_tracks_the_final_target_and_tls_but_excludes_passwords() {
        let a = connection("mysql://reader:one@db.example/analytics");
        let b = connection("mysql://reader:two@db.example:3306/analytics?ssl-mode=PREFERRED");
        let c = connection("mysql://reader:two@db.example/analytics");
        assert_eq!(a.identity, c.identity);
        assert_eq!(a.identity, b.identity);
        assert_ne!(
            b.identity,
            connection("mysql://reader:two@db.example/analytics?ssl-mode=DISABLED").identity
        );
        assert_ne!(
            a.identity,
            connection("mysql://reader:one@other.example/analytics").identity
        );
    }
}
