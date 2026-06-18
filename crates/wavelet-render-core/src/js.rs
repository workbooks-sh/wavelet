//! The in-wasm browser JS layer (Layer 3). Runs a page's `<script>`s in Boa (pure-Rust JS engine)
//! against the Blitz DOM: a `document` object whose methods (`getElementById`/`querySelector`/`body`)
//! return element handles whose `innerHTML`/`textContent` setters mutate the live Blitz DOM. Single-
//! threaded wasm, so a thread-local op queue lets fn-pointer natives record mutations without Boa
//! capture/GC plumbing; the queue is applied after the scripts run, then the doc re-resolves.

use crate::BaseDocument;
use blitz_dom::node::NodeData;
use blitz_html::HtmlDocument;
use boa_engine::object::ObjectInitializer;
use boa_engine::object::builtins::JsFunction;
use boa_engine::property::Attribute;
use boa_engine::{js_string, Context, JsObject, JsResult, JsValue, NativeFunction, Source};
use blitz_dom::local_name;
use std::cell::RefCell;

thread_local! {
    static OPS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

fn str_arg(args: &[JsValue], i: usize) -> String {
    args.get(i)
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default()
}

fn this_target(this: &JsValue, ctx: &mut Context) -> String {
    this.as_object()
        .and_then(|o| o.get(js_string!("__target"), ctx).ok())
        .and_then(|v| v.as_string().map(|s| s.to_std_string_escaped()))
        .unwrap_or_default()
}

// element.innerHTML = html  /  element.textContent = text  → queue (target, html)
fn set_html(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let target = this_target(this, ctx);
    let html = str_arg(args, 0);
    OPS.with(|o| o.borrow_mut().push((target, html)));
    Ok(JsValue::undefined())
}

fn jsfn(ctx: &mut Context, f: NativeFunction) -> JsFunction {
    f.to_js_function(ctx.realm())
}

fn make_element(ctx: &mut Context, target: &str) -> JsObject {
    let setter = jsfn(ctx, NativeFunction::from_fn_ptr(set_html));
    ObjectInitializer::new(ctx)
        .property(js_string!("__target"), js_string!(target), Attribute::all())
        .accessor(js_string!("innerHTML"), None, Some(setter.clone()), Attribute::all())
        .accessor(js_string!("textContent"), None, Some(setter), Attribute::all())
        .build()
}

fn get_element_by_id(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(make_element(ctx, &format!("#{}", str_arg(args, 0))).into())
}

fn query_selector(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    Ok(make_element(ctx, &str_arg(args, 0)).into())
}

fn console_log(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let parts: Vec<String> = args
        .iter()
        .map(|v| v.to_string(ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default())
        .collect();
    eprintln!("[js] {}", parts.join(" "));
    Ok(JsValue::undefined())
}

/// Collect inline `<script>` contents (skip `src=` external scripts) in document order.
pub fn collect_scripts(doc: &BaseDocument) -> Vec<String> {
    let mut out = Vec::new();
    walk_scripts(doc, doc.root_node().id, &mut out);
    out
}

fn walk_scripts(doc: &BaseDocument, id: usize, out: &mut Vec<String>) {
    let node = match doc.get_node(id) {
        Some(n) => n,
        None => return,
    };
    if let NodeData::Element(_) = &node.data {
        let name = node.element_data().map(|e| e.name.local.as_ref().to_string()).unwrap_or_default();
        if name == "script" {
            if node.attr(local_name!("src")).is_none() {
                out.push(node.text_content());
            }
            return;
        }
    }
    for c in &node.children {
        walk_scripts(doc, *c, out);
    }
}

/// Run the page's inline scripts against the Blitz DOM, applying their DOM mutations, then re-resolve.
pub fn run_scripts(doc: &mut HtmlDocument) {
    let scripts = collect_scripts(doc.as_ref());
    if scripts.is_empty() {
        return;
    }

    OPS.with(|o| o.borrow_mut().clear());

    {
        let mut ctx = Context::default();
        let body = make_element(&mut ctx, "body");
        let document = ObjectInitializer::new(&mut ctx)
            .property(js_string!("body"), body, Attribute::all())
            .function(NativeFunction::from_fn_ptr(get_element_by_id), js_string!("getElementById"), 1)
            .function(NativeFunction::from_fn_ptr(query_selector), js_string!("querySelector"), 1)
            .build();
        let _ = ctx.register_global_property(js_string!("document"), document, Attribute::all());

        let console = ObjectInitializer::new(&mut ctx)
            .function(NativeFunction::from_fn_ptr(console_log), js_string!("log"), 1)
            .build();
        let _ = ctx.register_global_property(js_string!("console"), console, Attribute::all());

        for s in &scripts {
            if let Err(e) = ctx.eval(Source::from_bytes(s)) {
                eprintln!("[js] script error: {e}");
            }
        }
    }

    let ops: Vec<(String, String)> = OPS.with(|o| o.borrow_mut().drain(..).collect());
    for (target, html) in ops {
        if let Some(id) = resolve_target(doc.as_ref(), &target) {
            doc.as_mut().mutate().set_inner_html(id, &html);
        }
    }
    // re-style + re-layout the mutated tree (like load_html_with_base: drain messages, resolve).
    for _ in 0..4 {
        doc.as_mut().handle_messages();
        doc.as_mut().resolve(0.0);
    }
}

fn resolve_target(doc: &BaseDocument, target: &str) -> Option<usize> {
    if target == "body" {
        find_element(doc, doc.root_node().id, "body")
    } else if let Some(id) = target.strip_prefix('#') {
        doc.get_element_by_id(id)
    } else {
        doc.query_selector(target).ok().flatten()
    }
}

fn find_element(doc: &BaseDocument, id: usize, name: &str) -> Option<usize> {
    let node = doc.get_node(id)?;
    if node.element_data().map(|e| e.name.local.as_ref() == name).unwrap_or(false) {
        return Some(id);
    }
    for c in &node.children {
        if let Some(f) = find_element(doc, *c, name) {
            return Some(f);
        }
    }
    None
}
