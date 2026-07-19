use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use std::convert::Infallible;
use std::net::SocketAddr;
use log::info;

pub async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    info!("🌐 HTTP connection");
    
    // Log da requisição
    info!("📩 Request: {} {}", req.method(), req.uri().path());
    
    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/plain")
        .header("Content-Length", "12")
        .header("Connection", "keep-alive")
        .body(Body::from("Hello World!"))
        .unwrap();
    
    Ok(response)
}

pub async fn run_server() -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    
    let make_svc = make_service_fn(|_conn| {
        async {
            Ok::<_, Infallible>(service_fn(handle))
        }
    });

    let server = Server::bind(&addr).serve(make_svc);
    
    info!("🚀 Server running on http://{}", addr);
    
    if let Err(e) = server.await {
        eprintln!("Server error: {}", e);
    }
    
    Ok(())
}
