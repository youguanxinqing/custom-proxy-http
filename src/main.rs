use std::fmt::Display;

use actix_web::{
    App, HttpRequest, HttpResponse, HttpResponseBuilder, HttpServer, ResponseError,
    http::{
        self,
        header::{self},
    },
    web,
};
use clap::Parser;
use futures::StreamExt;
use futures::channel::mpsc;
use log::{debug, info};
use tracing::instrument;

#[instrument(skip(payload))]
async fn proxy_api(
    req: HttpRequest,
    method: http::Method,
    mut payload: web::Payload,
    http_client: web::Data<reqwest::Client>,
) -> Result<HttpResponse, Error> {
    // 1. build proxied request
    // url query where stores proxied url
    let target_url = req.uri().query();
    if target_url.is_none() {
        return Ok(HttpResponse::BadRequest().body(format!("bad url format: {}", req.full_url())));
    }

    let mut target_headers = reqwest::header::HeaderMap::new();
    req.headers()
        .iter()
        .filter(|(key, _)| *key != "host")
        .for_each(|(key, val)| {
            target_headers.insert(
                reqwest::header::HeaderName::from_bytes(key.as_str().as_bytes()).unwrap(),
                reqwest::header::HeaderValue::from_bytes(val.as_bytes()).unwrap(),
            );
        });

    let (tx, rx) = mpsc::unbounded();
    actix_web::rt::spawn(async move {
        while let Some(chunk) = payload.next().await {
            tx.unbounded_send(chunk).unwrap();
        }
    });

    // 2. send proxied request to target server
    let target_resp = http_client
        .request(
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap(),
            target_url.unwrap().to_string(),
        )
        .headers(target_headers)
        .body(reqwest::Body::wrap_stream(rx))
        .send()
        .await
        .map_err(|err| anyhow::anyhow!("{}", err))?;

    // 3. build resp
    let mut resp = HttpResponseBuilder::new(
        http::StatusCode::from_u16(target_resp.status().as_u16())
            .map_err(|err| anyhow::anyhow!("{}", err))?,
    );
    // set headers
    target_resp
        .headers()
        .iter()
        .filter(|(key, _)| *key != "connection")
        .for_each(|(key, val)| {
            resp.append_header((
                header::HeaderName::from_bytes(key.as_str().as_bytes()).unwrap(),
                header::HeaderValue::from_bytes(val.as_bytes()).unwrap(),
            ));
        });
    Ok(resp.streaming(target_resp.bytes_stream()))
}

#[derive(Debug, Clone, Parser)]
struct CommandArgs {
    #[clap(long, default_value = "127.0.0.1", help = "specify listen host")]
    host: String,

    #[clap(long, default_value_t = 8080, help = "specify listen port")]
    port: u16,

    #[clap(long, default_value_t = 10, help = "specify workers number")]
    workers: u8,
}

#[derive(Debug)]
struct Error(anyhow::Error);

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.0))
    }
}

impl ResponseError for Error {}

impl From<anyhow::Error> for Error {
    fn from(value: anyhow::Error) -> Self {
        Self(value)
    }
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    tracing_subscriber::fmt().finish();

    let args = CommandArgs::parse();
    debug!("command args: {:?}", args);

    info!(
        "start custom http proxy server, listen addr {}:{}",
        args.host, args.port
    );

    let http_client = reqwest::Client::new();

    let _ = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(http_client.clone()))
            .service(web::scope("/proxy").default_service(web::to(proxy_api)))
    })
    .bind((args.host, args.port))?
    .workers(args.workers.into())
    .run()
    .await;

    Ok(())
}
