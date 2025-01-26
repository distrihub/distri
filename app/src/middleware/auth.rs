use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    web, Error, HttpMessage,
};
use agent_twitter_client::models::Profile;
use diesel::prelude::*;
use futures_util::future::LocalBoxFuture;
use std::{
    future::{ready, Ready},
    rc::Rc,
};

use crate::{db::DbPool, handlers::x::validate_session, models::session::Session};

pub struct AuthMiddleware;

#[derive(Clone)]
pub struct AuthSession {
    pub profile: Profile,
    pub session: Session,
}

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddlewareService {
            service: Rc::new(service),
        }))
    }
}

#[derive(Clone)]
pub struct AuthMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Rc::clone(&self.service);
        Box::pin(async move {
            let pool = req.app_data::<web::Data<DbPool>>().unwrap().clone();

            // First try to get token from Authorization header, then from cookie
            let token = req
                .headers()
                .get("Authorization")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer ").map(|s| s.to_string()))
                .or_else(|| req.cookie("auth_token").map(|c| c.value().to_string()));

            if let Some(token) = token {
                let mut conn = pool.get().expect("couldn't get db connection from pool");

                // Verify session token
                use crate::schema::sessions::dsl::*;
                match sessions
                    .filter(session_token.eq(token))
                    .filter(expires_at.gt(diesel::dsl::now))
                    .select(Session::as_select())
                    .first(&mut conn)
                {
                    Ok(session) => {
                        let auth_session = validate_session(session).await.map_err(|e| {
                            actix_web::error::ErrorInternalServerError(e.to_string())
                        })?;
                        req.extensions_mut().insert(auth_session);
                        service.call(req).await
                    }
                    Err(_) => Err(actix_web::error::ErrorUnauthorized("Invalid session")),
                }
            } else {
                Err(actix_web::error::ErrorUnauthorized(
                    "No session token provided",
                ))
            }
        })
    }
}
