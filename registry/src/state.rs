use crate::config::AppConfig;
use crate::storage::r2::R2Client;
use sqlx::PgPool;
use std::sync::Arc;

/// The GitHub OAuth client, with its authorization and token endpoints set.
pub type GithubOAuthClient = oauth2::basic::BasicClient<
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub s3: R2Client,
    pub config: Arc<AppConfig>,
    pub oauth_client: Arc<GithubOAuthClient>,
}
