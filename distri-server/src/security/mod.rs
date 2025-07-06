use actix_web::{
    body::EitherBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    error::ErrorUnauthorized,
    Error, FromRequest, HttpMessage, HttpRequest,
};
use distri::types::ServerConfig;
use distri_a2a::SecurityScheme;
use futures_util::{future::LocalBoxFuture, FutureExt};
use std::{
    collections::HashMap,
    future::{ready, Ready},
    sync::Arc,
};

pub mod validators;

#[cfg(test)]
mod tests;

pub use validators::*;

/// Security context that is attached to requests after authentication
#[derive(Debug, Clone)]
pub struct SecurityContext {
    pub user_id: Option<String>,
    pub scopes: Vec<String>,
    pub scheme_name: String,
}

impl SecurityContext {
    pub fn new(user_id: Option<String>, scopes: Vec<String>, scheme_name: String) -> Self {
        Self {
            user_id,
            scopes,
            scheme_name,
        }
    }
}

/// Security middleware that validates requests based on configured security schemes
#[derive(Clone)]
pub struct SecurityMiddleware {
    security_schemes: HashMap<String, SecurityScheme>,
    security_requirements: Vec<HashMap<String, Vec<String>>>,
    protected_paths: Vec<String>,
}

impl SecurityMiddleware {
    pub fn new(
        security_schemes: HashMap<String, SecurityScheme>,
        security_requirements: Vec<HashMap<String, Vec<String>>>,
    ) -> Self {
        // Define paths that require authentication
        let protected_paths = vec![
            "/api/v1/agents".to_string(),
            "/api/v1/tasks".to_string(),
            "/api/v1/threads".to_string(),
        ];

        Self {
            security_schemes,
            security_requirements,
            protected_paths,
        }
    }

    fn requires_authentication(&self, path: &str) -> bool {
        self.protected_paths.iter().any(|p| path.starts_with(p))
    }

    async fn validate_request(&self, req: &ServiceRequest) -> Result<SecurityContext, Error> {
        let path = req.path();
        
        // Skip authentication for non-protected paths
        if !self.requires_authentication(path) {
            return Ok(SecurityContext::new(None, vec![], "none".to_string()));
        }

        // If no security schemes are configured, allow all requests
        if self.security_schemes.is_empty() {
            return Ok(SecurityContext::new(None, vec![], "none".to_string()));
        }

        // Try to authenticate using any of the configured schemes
        for (scheme_name, scheme) in &self.security_schemes {
            if let Ok(context) = self.validate_scheme(req, scheme_name, scheme).await {
                return Ok(context);
            }
        }

        Err(ErrorUnauthorized("Authentication required"))
    }

    async fn validate_scheme(
        &self,
        req: &ServiceRequest,
        scheme_name: &str,
        scheme: &SecurityScheme,
    ) -> Result<SecurityContext, Error> {
        match scheme {
            SecurityScheme::ApiKey(api_key_scheme) => {
                validate_api_key(req, scheme_name, api_key_scheme).await
            }
            SecurityScheme::Http(http_scheme) => {
                validate_http_auth(req, scheme_name, http_scheme).await
            }
            SecurityScheme::Oauth2(_) => {
                // OAuth2 validation would be implemented here
                Err(ErrorUnauthorized("OAuth2 not implemented yet"))
            }
            SecurityScheme::OpenIdConnect(_) => {
                // OpenID Connect validation would be implemented here
                Err(ErrorUnauthorized("OpenID Connect not implemented yet"))
            }
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for SecurityMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = SecurityMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SecurityMiddlewareService {
            service: Arc::new(service),
            security_schemes: self.security_schemes.clone(),
            security_requirements: self.security_requirements.clone(),
            protected_paths: self.protected_paths.clone(),
        }))
    }
}

pub struct SecurityMiddlewareService<S> {
    service: Arc<S>,
    security_schemes: HashMap<String, SecurityScheme>,
    security_requirements: Vec<HashMap<String, Vec<String>>>,
    protected_paths: Vec<String>,
}

impl<S, B> Service<ServiceRequest> for SecurityMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let middleware = SecurityMiddleware::new(
            self.security_schemes.clone(),
            self.security_requirements.clone(),
        );

        let service = self.service.clone();

        async move {
            // Validate the request
            match middleware.validate_request(&req).await {
                Ok(security_context) => {
                    // Add security context to request extensions
                    req.extensions_mut().insert(security_context);
                    
                    // Continue with the request
                    let res = service.call(req).await?;
                    Ok(res.map_into_left_body())
                }
                Err(err) => {
                    // Return authorization error
                    Err(err)
                }
            }
        }
        .boxed_local()
    }
}

/// Extractor for security context from request
impl FromRequest for SecurityContext {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        if let Some(context) = req.extensions().get::<SecurityContext>() {
            ready(Ok(context.clone()))
        } else {
            ready(Ok(SecurityContext::new(None, vec![], "none".to_string())))
        }
    }
}

/// Helper function to create security middleware from server config
pub fn create_security_middleware(server_config: &ServerConfig) -> SecurityMiddleware {
    SecurityMiddleware::new(
        server_config.security_schemes.clone(),
        server_config.security.clone(),
    )
}