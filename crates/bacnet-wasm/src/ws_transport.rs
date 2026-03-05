//! Browser WebSocket adapter for BACnet/SC.
//!
//! Wraps the `web-sys::WebSocket` API into an async Rust interface.
//! Uses `Rc<RefCell<>>` since WASM is single-threaded.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use js_sys::{ArrayBuffer, Promise, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{BinaryType, CloseEvent, ErrorEvent, MessageEvent, WebSocket};

/// Wraps a browser WebSocket for BACnet/SC binary framing.
pub struct BrowserWebSocket {
    ws: WebSocket,
    recv_queue: Rc<RefCell<VecDeque<Vec<u8>>>>,
    recv_waker: Rc<RefCell<Option<std::task::Waker>>>,
    error: Rc<RefCell<Option<String>>>,
}

impl BrowserWebSocket {
    /// Open a WebSocket connection to a BACnet/SC hub.
    pub async fn connect(url: &str) -> Result<Self, JsValue> {
        let ws = WebSocket::new_with_str(url, "hub.bsc.bacnet.org")?;
        ws.set_binary_type(BinaryType::Arraybuffer);

        let recv_queue = Rc::new(RefCell::new(VecDeque::new()));
        let recv_waker: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
        let error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        // Wait for open using a JS Promise
        let open_promise = Promise::new(&mut |resolve, reject| {
            let reject_clone = reject.clone();
            let onopen = Closure::<dyn FnMut()>::once(move || {
                let _ = resolve.call0(&JsValue::NULL);
            });
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            onopen.forget();

            let onerror = Closure::<dyn FnMut(ErrorEvent)>::once(move |_e: ErrorEvent| {
                let _ = reject_clone.call1(
                    &JsValue::NULL,
                    &JsValue::from_str("WebSocket connection failed"),
                );
            });
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            onerror.forget();
        });

        JsFuture::from(open_promise).await?;

        // Set up persistent callbacks after connection
        // onerror (post-connect)
        {
            let error = error.clone();
            let waker = recv_waker.clone();
            let onerror = Closure::<dyn FnMut(ErrorEvent)>::new(move |_e: ErrorEvent| {
                *error.borrow_mut() = Some("WebSocket error".into());
                if let Some(w) = waker.borrow_mut().take() {
                    w.wake();
                }
            });
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            onerror.forget();
        }

        // onmessage
        {
            let queue = recv_queue.clone();
            let waker = recv_waker.clone();
            let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
                if let Ok(buf) = e.data().dyn_into::<ArrayBuffer>() {
                    let array = Uint8Array::new(&buf);
                    let data = array.to_vec();
                    queue.borrow_mut().push_back(data);
                    if let Some(w) = waker.borrow_mut().take() {
                        w.wake();
                    }
                }
            });
            ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
            onmessage.forget();
        }

        // onclose
        {
            let error = error.clone();
            let waker = recv_waker.clone();
            let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |e: CloseEvent| {
                *error.borrow_mut() = Some(format!("WebSocket closed: code={}", e.code()));
                if let Some(w) = waker.borrow_mut().take() {
                    w.wake();
                }
            });
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
            onclose.forget();
        }

        Ok(Self {
            ws,
            recv_queue,
            recv_waker,
            error,
        })
    }

    /// Send binary data.
    pub fn send(&self, data: &[u8]) -> Result<(), JsValue> {
        self.ws.send_with_u8_array(data)
    }

    /// Receive the next binary message. Suspends until data is available.
    pub async fn recv(&self) -> Result<Vec<u8>, String> {
        RecvFuture {
            queue: self.recv_queue.clone(),
            waker: self.recv_waker.clone(),
            error: self.error.clone(),
        }
        .await
    }

    /// Close the WebSocket connection.
    pub fn close(&self) {
        let _ = self.ws.close();
    }
}

/// Future that resolves when a message arrives or an error occurs.
struct RecvFuture {
    queue: Rc<RefCell<VecDeque<Vec<u8>>>>,
    waker: Rc<RefCell<Option<std::task::Waker>>>,
    error: Rc<RefCell<Option<String>>>,
}

impl std::future::Future for RecvFuture {
    type Output = Result<Vec<u8>, String>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if let Some(data) = self.queue.borrow_mut().pop_front() {
            return std::task::Poll::Ready(Ok(data));
        }
        if let Some(err) = self.error.borrow().clone() {
            return std::task::Poll::Ready(Err(err));
        }
        *self.waker.borrow_mut() = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}
