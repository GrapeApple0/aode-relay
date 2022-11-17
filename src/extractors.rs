use actix_web::{
    dev::Payload,
    error::ParseError,
    http::{
        header::{from_one_raw_str, Header, HeaderName, HeaderValue, TryIntoHeaderValue},
        StatusCode,
    },
    web::Data,
    FromRequest, HttpMessage, HttpRequest, HttpResponse, ResponseError,
};
use bcrypt::{BcryptError, DEFAULT_COST};
use http_signature_normalization_actix::prelude::InvalidHeaderValue;
use std::{
    convert::Infallible,
    future::{ready, Ready},
    str::FromStr,
};
use tracing_error::SpanTrace;

use crate::db::Db;

#[derive(Clone)]
pub(crate) struct AdminConfig {
    hashed_api_token: String,
}

impl AdminConfig {
    pub(crate) fn build(api_token: &str) -> Result<Self, Error> {
        Ok(AdminConfig {
            hashed_api_token: bcrypt::hash(api_token, DEFAULT_COST).map_err(Error::bcrypt_hash)?,
        })
    }

    fn verify(&self, token: XApiToken) -> Result<bool, Error> {
        Ok(bcrypt::verify(&self.hashed_api_token, &token.0).map_err(Error::bcrypt_verify)?)
    }
}

pub(crate) struct Admin {
    db: Data<Db>,
}

impl Admin {
    #[tracing::instrument(level = "debug", skip(req))]
    fn verify(req: &HttpRequest) -> Result<Self, Error> {
        let hashed_api_token = req
            .app_data::<Data<AdminConfig>>()
            .ok_or_else(Error::missing_config)?;

        let x_api_token = XApiToken::parse(req).map_err(Error::parse_header)?;

        if hashed_api_token.verify(x_api_token)? {
            let db = req.app_data::<Data<Db>>().ok_or_else(Error::missing_db)?;

            return Ok(Self { db: db.clone() });
        }

        Err(Error::invalid())
    }

    pub(crate) fn db_ref(&self) -> &Db {
        &self.db
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Failed authentication")]
pub(crate) struct Error {
    context: SpanTrace,
    #[source]
    kind: ErrorKind,
}

impl Error {
    fn invalid() -> Self {
        Error {
            context: SpanTrace::capture(),
            kind: ErrorKind::Invalid,
        }
    }

    fn missing_config() -> Self {
        Error {
            context: SpanTrace::capture(),
            kind: ErrorKind::MissingConfig,
        }
    }

    fn missing_db() -> Self {
        Error {
            context: SpanTrace::capture(),
            kind: ErrorKind::MissingDb,
        }
    }

    fn bcrypt_verify(e: BcryptError) -> Self {
        Error {
            context: SpanTrace::capture(),
            kind: ErrorKind::BCryptVerify(e),
        }
    }

    fn bcrypt_hash(e: BcryptError) -> Self {
        Error {
            context: SpanTrace::capture(),
            kind: ErrorKind::BCryptHash(e),
        }
    }

    fn parse_header(e: ParseError) -> Self {
        Error {
            context: SpanTrace::capture(),
            kind: ErrorKind::ParseHeader(e),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error("Invalid API Token")]
    Invalid,

    #[error("Missing Config")]
    MissingConfig,

    #[error("Missing Db")]
    MissingDb,

    #[error("Verifying")]
    BCryptVerify(#[source] BcryptError),

    #[error("Hashing")]
    BCryptHash(#[source] BcryptError),

    #[error("Parse Header")]
    ParseHeader(#[source] ParseError),
}

impl ResponseError for Error {
    fn status_code(&self) -> StatusCode {
        match self.kind {
            ErrorKind::Invalid | ErrorKind::ParseHeader(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code())
            .json(serde_json::json!({ "msg": self.kind.to_string() }))
    }
}

impl FromRequest for Admin {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(Admin::verify(req))
    }
}

pub(crate) struct XApiToken(String);

impl XApiToken {
    pub(crate) fn new(token: String) -> Self {
        Self(token)
    }
}

impl Header for XApiToken {
    fn name() -> HeaderName {
        HeaderName::from_static("x-api-token")
    }

    fn parse<M: HttpMessage>(msg: &M) -> Result<Self, ParseError> {
        from_one_raw_str(msg.headers().get(Self::name()))
    }
}

impl TryIntoHeaderValue for XApiToken {
    type Error = InvalidHeaderValue;

    fn try_into_value(self) -> Result<HeaderValue, Self::Error> {
        HeaderValue::from_str(&self.0)
    }
}

impl FromStr for XApiToken {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(XApiToken(s.to_string()))
    }
}
