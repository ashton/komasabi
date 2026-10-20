use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// The request failed or the server answered with an error status.
    Http(reqwest::Error),
    /// A URL could not be parsed or resolved.
    Url(url::ParseError),
    /// An OPDS 2 (JSON) document could not be decoded.
    Json(serde_json::Error),
    /// An OPDS 1.x (Atom XML) document could not be decoded.
    Xml(String),
    /// A download could not be written to disk.
    Io(std::io::Error),
    /// The response was neither an OPDS 2 nor an OPDS 1.x document.
    UnsupportedFormat(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http(error) => write!(f, "http error: {error}"),
            Error::Url(error) => write!(f, "invalid url: {error}"),
            Error::Json(error) => write!(f, "invalid OPDS 2 document: {error}"),
            Error::Xml(error) => write!(f, "invalid OPDS 1 document: {error}"),
            Error::Io(error) => write!(f, "could not write download: {error}"),
            Error::UnsupportedFormat(content_type) => {
                write!(f, "not an OPDS document (content type: {content_type})")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Error::Http(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error)
    }
}

impl From<url::ParseError> for Error {
    fn from(error: url::ParseError) -> Self {
        Error::Url(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Error::Json(error)
    }
}

impl From<quick_xml::Error> for Error {
    fn from(error: quick_xml::Error) -> Self {
        Error::Xml(error.to_string())
    }
}
