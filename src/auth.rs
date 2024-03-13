use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{async_trait, Json, RequestPartsExt};
use axum_extra::TypedHeader;
use headers::authorization::Bearer;
use headers::Authorization;
use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, TokenData, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::str::FromStr;

pub enum AuthError {
    MissingToken,
    TokenError,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "missing jwt"),
            AuthError::TokenError => (
                StatusCode::FORBIDDEN,
                "incorrect jwt or missing permissions",
            ),
        };
        let body = Json(json!({
            "error": error,
        }));
        (status, body).into_response()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub exp: usize,
}

#[async_trait]
impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::MissingToken)?;

        let token_data = parse_jwt(bearer.token())
            .await
            .map_err(|_| AuthError::TokenError)?;

        Ok(token_data.claims)
    }
}

async fn parse_jwt(token: &str) -> Result<TokenData<Claims>, Box<dyn Error>> {
    let authority = std::env::var("AUTH0_ISSUER").unwrap();
    let jwks = fetch_jwks(&format!("{}.well-known/jwks.json", authority.as_str()))
        .await
        .unwrap();

    let header = decode_header(token)?;
    let kid = match header.kid {
        Some(k) => k,
        None => return Err("Token doesn't have a `kid` header field".into()),
    };
    if let Some(j) = jwks.find(&kid) {
        match &j.algorithm {
            AlgorithmParameters::RSA(rsa) => {
                let decoding_key = DecodingKey::from_rsa_components(&rsa.n, &rsa.e).unwrap();

                let mut validation = Validation::new(
                    Algorithm::from_str(j.common.key_algorithm.unwrap().to_string().as_str())
                        .unwrap(),
                );
                validation.set_audience(&["https://bitsandbytesbooks.backend.com"]);
                validation.set_issuer(&["https://dev-11u3ws3u.us.auth0.com/"]);
                validation.validate_exp = false;
                let decoded_token = decode::<Claims>(token, &decoding_key, &validation).unwrap();
                Ok(decoded_token)
            }
            _ => unreachable!("this should be a RSA"),
        }
    } else {
        Err("No matching JWK found for the given kid".into())
    }
}

async fn fetch_jwks(uri: &str) -> Result<JwkSet, Box<dyn Error>> {
    let res = reqwest::get(uri).await?;
    let val = res.json::<JwkSet>().await?;
    Ok(val)
}
