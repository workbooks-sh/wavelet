//! The in-wasm browser JS layer (Layer 3). Runs a page's `<script>`s in Boa (pure-Rust JS engine)
//! against a LIVE Blitz DOM, in wasmtime. Each JS element handle wraps a live Blitz node id; the
//! native DOM functions read + mutate the live `BaseDocument` during script execution. Single-
//! threaded wasm makes a thread-local doc pointer sound — the pointer is valid for the eval duration.

use crate::BaseDocument;
use blitz_dom::local_name;
use blitz_dom::node::NodeData;
use blitz_dom::{ns, LocalName, QualName};
use blitz_html::HtmlDocument;
use boa_engine::object::builtins::{JsArray, JsFunction};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{js_string, Context, JsObject, JsResult, JsValue, NativeFunction, Source};
use std::cell::Cell;
use std::ptr;

thread_local! {
    static DOC: Cell<*mut BaseDocument> = const { Cell::new(ptr::null_mut()) };
}

fn with_doc<R>(f: impl FnOnce(&mut BaseDocument) -> R) -> Option<R> {
    let p = DOC.with(|c| c.get());
    if p.is_null() {
        None
    } else {
        Some(unsafe { f(&mut *p) })
    }
}

fn str_arg(args: &[JsValue], i: usize) -> String {
    args.get(i)
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_default()
}

fn this_nid(this: &JsValue, ctx: &mut Context) -> Option<usize> {
    let v = this.as_object()?.get(js_string!("__nodeid"), ctx).ok()?;
    v.as_number().map(|n| n as usize)
}

fn arg_nid(args: &[JsValue], i: usize, ctx: &mut Context) -> Option<usize> {
    let v = args.get(i)?.as_object()?.get(js_string!("__nodeid"), ctx).ok()?;
    v.as_number().map(|n| n as usize)
}

fn html_qual(name: &str) -> QualName {
    QualName::new(None, ns!(html), LocalName::from(name.to_ascii_lowercase()))
}

fn jsfn(ctx: &mut Context, f: fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>) -> JsFunction {
    NativeFunction::from_fn_ptr(f).to_js_function(ctx.realm())
}

// ── element handle ─────────────────────────────────────────────────────────

/// Build a JS element handle wrapping live Blitz node `nid` — its accessors/methods operate on the
/// live DOM. `null` for a missing node.
fn handle(ctx: &mut Context, nid: Option<usize>) -> JsValue {
    let Some(nid) = nid else { return JsValue::null() };
    let inner_html = jsfn(ctx, el_set_inner_html);
    let text_get = jsfn(ctx, el_get_text);
    let text_set = jsfn(ctx, el_set_text);
    let id_get = jsfn(ctx, el_get_id);
    let id_set = jsfn(ctx, el_set_id);
    let class_get = jsfn(ctx, el_get_class);
    let class_set = jsfn(ctx, el_set_class);
    let tag_get = jsfn(ctx, el_get_tag);

    ObjectInitializer::new(ctx)
        .property(js_string!("__nodeid"), JsValue::from(nid as i32), Attribute::all())
        .accessor(js_string!("innerHTML"), None, Some(inner_html), Attribute::all())
        .accessor(js_string!("textContent"), Some(text_get), Some(text_set), Attribute::all())
        .accessor(js_string!("id"), Some(id_get), Some(id_set), Attribute::all())
        .accessor(js_string!("className"), Some(class_get), Some(class_set), Attribute::all())
        .accessor(js_string!("tagName"), Some(tag_get), None, Attribute::all())
        .function(NativeFunction::from_fn_ptr(el_set_attr), js_string!("setAttribute"), 2)
        .function(NativeFunction::from_fn_ptr(el_get_attr), js_string!("getAttribute"), 1)
        .function(NativeFunction::from_fn_ptr(el_append_child), js_string!("appendChild"), 1)
        .function(NativeFunction::from_fn_ptr(el_remove), js_string!("remove"), 0)
        .function(NativeFunction::from_fn_ptr(el_query_selector), js_string!("querySelector"), 1)
        .function(NativeFunction::from_fn_ptr(el_query_selector_all), js_string!("querySelectorAll"), 1)
        .function(NativeFunction::from_fn_ptr(noop), js_string!("addEventListener"), 2)
        .function(NativeFunction::from_fn_ptr(noop), js_string!("removeEventListener"), 2)
        .build()
        .into()
}

fn el_set_inner_html(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if let Some(nid) = this_nid(this, ctx) {
        let html = str_arg(args, 0);
        with_doc(|d| d.mutate().set_inner_html(nid, &html));
    }
    Ok(JsValue::undefined())
}

fn el_get_text(this: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = this_nid(this, ctx)
        .and_then(|nid| with_doc(|d| d.get_node(nid).map(|n| n.text_content())).flatten())
        .unwrap_or_default();
    Ok(js_string!(t).into())
}

fn el_set_text(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if let Some(nid) = this_nid(this, ctx) {
        let text = str_arg(args, 0);
        with_doc(|d| {
            let mut m = d.mutate();
            m.remove_and_drop_all_children(nid);
            let t = m.create_text_node(&text);
            m.append_children(nid, &[t]);
        });
    }
    Ok(JsValue::undefined())
}

fn el_set_attr(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if let Some(nid) = this_nid(this, ctx) {
        let (name, val) = (str_arg(args, 0), str_arg(args, 1));
        with_doc(|d| d.mutate().set_attribute(nid, html_qual(&name), &val));
    }
    Ok(JsValue::undefined())
}

fn el_get_attr(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = str_arg(args, 0);
    let v = this_nid(this, ctx).and_then(|nid| {
        with_doc(|d| {
            d.get_node(nid)
                .and_then(|n| n.attr(LocalName::from(name.to_ascii_lowercase())).map(|s| s.to_string()))
        })
        .flatten()
    });
    Ok(v.map(|s| js_string!(s).into()).unwrap_or(JsValue::null()))
}

fn el_append_child(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if let (Some(parent), Some(child)) = (this_nid(this, ctx), arg_nid(args, 0, ctx)) {
        with_doc(|d| d.mutate().append_children(parent, &[child]));
    }
    Ok(args.first().cloned().unwrap_or(JsValue::undefined()))
}

fn el_remove(this: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    if let Some(nid) = this_nid(this, ctx) {
        with_doc(|d| d.mutate().remove_node(nid));
    }
    Ok(JsValue::undefined())
}

fn el_get_id(this: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    get_attr_named(this, "id", ctx)
}
fn el_set_id(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_attr_named(this, "id", &str_arg(args, 0), ctx)
}
fn el_get_class(this: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    get_attr_named(this, "class", ctx)
}
fn el_set_class(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    set_attr_named(this, "class", &str_arg(args, 0), ctx)
}

fn el_get_tag(this: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let t = this_nid(this, ctx)
        .and_then(|nid| {
            with_doc(|d| {
                d.get_node(nid)
                    .and_then(|n| n.element_data().map(|e| e.name.local.as_ref().to_uppercase()))
            })
            .flatten()
        })
        .unwrap_or_default();
    Ok(js_string!(t).into())
}

fn el_query_selector(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let _ = this;
    let sel = str_arg(args, 0);
    let nid = with_doc(|d| d.query_selector(&sel).ok().flatten()).flatten();
    Ok(handle(ctx, nid))
}

fn el_query_selector_all(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let _ = this;
    let sel = str_arg(args, 0);
    let ids = with_doc(|d| d.query_selector_all(&sel).unwrap_or_default()).unwrap_or_default();
    let arr = JsArray::new(ctx);
    for nid in ids {
        let h = handle(ctx, Some(nid));
        arr.push(h, ctx)?;
    }
    Ok(arr.into())
}

fn get_attr_named(this: &JsValue, name: &str, ctx: &mut Context) -> JsResult<JsValue> {
    let v = this_nid(this, ctx).and_then(|nid| {
        with_doc(|d| d.get_node(nid).and_then(|n| n.attr(LocalName::from(name)).map(|s| s.to_string()))).flatten()
    });
    Ok(js_string!(v.unwrap_or_default()).into())
}
fn set_attr_named(this: &JsValue, name: &str, val: &str, ctx: &mut Context) -> JsResult<JsValue> {
    if let Some(nid) = this_nid(this, ctx) {
        with_doc(|d| d.mutate().set_attribute(nid, html_qual(name), val));
    }
    Ok(JsValue::undefined())
}

fn noop(_t: &JsValue, _a: &[JsValue], _ctx: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

// ── document ───────────────────────────────────────────────────────────────

fn doc_create_element(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let tag = str_arg(args, 0);
    let nid = with_doc(|d| d.mutate().create_element(html_qual(&tag), vec![]));
    Ok(handle(ctx, nid))
}

fn doc_create_text_node(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = str_arg(args, 0);
    let nid = with_doc(|d| d.mutate().create_text_node(&text));
    Ok(handle(ctx, nid))
}

fn doc_get_by_id(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = str_arg(args, 0);
    let nid = with_doc(|d| d.get_element_by_id(&id)).flatten();
    Ok(handle(ctx, nid))
}

fn doc_query_selector(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let sel = str_arg(args, 0);
    let nid = with_doc(|d| d.query_selector(&sel).ok().flatten()).flatten();
    Ok(handle(ctx, nid))
}

fn doc_query_selector_all(this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    el_query_selector_all(this, args, ctx)
}

fn doc_body(_t: &JsValue, _a: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let nid = with_doc(|d| find_element(d, d.root_node().id, "body")).flatten();
    Ok(handle(ctx, nid))
}

fn console_log(_t: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let parts: Vec<String> = args
        .iter()
        .map(|v| v.to_string(ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default())
        .collect();
    eprintln!("[js] {}", parts.join(" "));
    Ok(JsValue::undefined())
}

// ── runner ─────────────────────────────────────────────────────────────────

/// Collect inline `<script>` contents (skip `src=` external) in document order.
pub fn collect_scripts(doc: &BaseDocument) -> Vec<String> {
    let mut out = Vec::new();
    walk_scripts(doc, doc.root_node().id, &mut out);
    out
}

fn walk_scripts(doc: &BaseDocument, id: usize, out: &mut Vec<String>) {
    let Some(node) = doc.get_node(id) else { return };
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

/// Run the page's inline scripts against the LIVE Blitz DOM, then re-resolve.
pub fn run_scripts(doc: &mut HtmlDocument) {
    let scripts = collect_scripts(doc.as_ref());
    if scripts.is_empty() {
        return;
    }

    DOC.with(|c| c.set(doc.as_mut() as *mut BaseDocument));

    {
        let mut ctx = Context::default();
        build_dom_globals(&mut ctx);
        for s in &scripts {
            if let Err(e) = ctx.eval(Source::from_bytes(s)) {
                eprintln!("[js] script error: {e}");
            }
        }
    }

    DOC.with(|c| c.set(ptr::null_mut()));

    for _ in 0..4 {
        doc.as_mut().handle_messages();
        doc.as_mut().resolve(0.0);
    }
}

fn build_dom_globals(ctx: &mut Context) {
    let body_get = jsfn(ctx, doc_body);
    let document = ObjectInitializer::new(ctx)
        .accessor(js_string!("body"), Some(body_get), None, Attribute::all())
        .function(NativeFunction::from_fn_ptr(doc_get_by_id), js_string!("getElementById"), 1)
        .function(NativeFunction::from_fn_ptr(doc_query_selector), js_string!("querySelector"), 1)
        .function(NativeFunction::from_fn_ptr(doc_query_selector_all), js_string!("querySelectorAll"), 1)
        .function(NativeFunction::from_fn_ptr(doc_create_element), js_string!("createElement"), 1)
        .function(NativeFunction::from_fn_ptr(doc_create_text_node), js_string!("createTextNode"), 1)
        .function(NativeFunction::from_fn_ptr(noop), js_string!("addEventListener"), 2)
        .build();
    let _ = ctx.register_global_property(js_string!("document"), document, Attribute::all());

    let console = ObjectInitializer::new(ctx)
        .function(NativeFunction::from_fn_ptr(console_log), js_string!("log"), 1)
        .function(NativeFunction::from_fn_ptr(console_log), js_string!("error"), 1)
        .function(NativeFunction::from_fn_ptr(console_log), js_string!("warn"), 1)
        .build();
    let _ = ctx.register_global_property(js_string!("console"), console, Attribute::all());

    // minimal window/location/navigator so framework boot code doesn't throw.
    let location = ObjectInitializer::new(ctx)
        .property(js_string!("href"), js_string!("about:blank"), Attribute::all())
        .property(js_string!("pathname"), js_string!("/"), Attribute::all())
        .build();
    let navigator = ObjectInitializer::new(ctx)
        .property(js_string!("userAgent"), js_string!("nexus-blitz"), Attribute::all())
        .build();
    let _ = ctx.register_global_property(js_string!("location"), location, Attribute::all());
    let _ = ctx.register_global_property(js_string!("navigator"), navigator, Attribute::all());
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
