//! User scripts and the framework's own page-side agent.

/// A script a view runs in every page it loads (in each frame with `all_frames`). Re-adding an
/// `id` replaces the script.
#[derive(Clone, Debug, PartialEq)]
pub struct UserScript {
    pub id: String,
    pub source: String,
    pub injection: Injection,
    /// A named isolated world, or `None` for the page's own.
    pub world: Option<String>,
    pub all_frames: bool,
    /// `*` globs over the page URL. A non-empty allow list runs the script only where one
    /// matches; the block list wins over it.
    pub allow_list: Vec<String>,
    pub block_list: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Injection {
    /// Before any of the page's scripts.
    Start,
    /// Once the document has parsed.
    #[default]
    End,
}

impl UserScript {
    pub fn new(id: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            injection: Injection::End,
            world: None,
            all_frames: false,
            allow_list: Vec::new(),
            block_list: Vec::new(),
        }
    }

    pub fn at_start(mut self) -> Self {
        self.injection = Injection::Start;
        self
    }

    pub fn in_world(mut self, world: impl Into<String>) -> Self {
        self.world = Some(world.into());
        self
    }

    pub fn all_frames(mut self) -> Self {
        self.all_frames = true;
        self
    }

    pub fn allow(mut self, globs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allow_list = globs.into_iter().map(Into::into).collect();
        self
    }

    pub fn block(mut self, globs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.block_list = globs.into_iter().map(Into::into).collect();
        self
    }

    /// The source wrapped in a URL guard when it has allow or block lists. Neither engine has
    /// per-script URL lists, so the guard runs in the page.
    pub fn guarded_source(&self) -> String {
        if self.allow_list.is_empty() && self.block_list.is_empty() {
            return self.source.clone();
        }
        let list = |l: &[String]| serde_json::to_string(l).unwrap_or_else(|_| "[]".into());
        format!(
            "(function(){{var a={},b={};function m(p,u){{return new RegExp('^'+p.split('*').map(function(s){{\
             return s.replace(/[.+?^${{}}()|[\\]\\\\]/g,'\\\\$&');}}).join('.*')+'$').test(u);}}\
             var u=location.href;if(b.some(function(p){{return m(p,u);}}))return;\
             if(a.length&&!a.some(function(p){{return m(p,u);}}))return;\n{}\n}})();",
            list(&self.allow_list),
            list(&self.block_list),
            self.source
        )
    }
}

/// The private world the framework's own agent runs in, and its message handler. Both names are
/// reserved.
pub const INTERNAL_WORLD: &str = "nd-internal";
pub const INTERNAL_HANDLER: &str = "__ndInternal";

/// The page-side agent for engines that do not report the hovered link, the context menu's hit
/// test, media playback or element fullscreen (WebKit): it posts `{k: ...}` messages to
/// [`INTERNAL_HANDLER`]. `post` is the expression that sends one message `m`.
pub fn agent_source(post: &str, suppress_menu: bool) -> String {
    format!(
        r#"(function(){{
var post=function(m){{try{{{post}}}catch(e){{}}}};
var lastHover="";
var linkOf=function(el){{while(el&&el!==document){{if(el.tagName==="A"&&el.href)return el.href;el=el.parentElement;}}return "";}};
document.addEventListener("mouseover",function(e){{var h=linkOf(e.target);if(h!==lastHover){{lastHover=h;post({{k:"hover",url:h}});}}}},true);
document.addEventListener("mouseout",function(e){{if(!e.relatedTarget&&lastHover!==""){{lastHover="";post({{k:"hover",url:""}});}}}},true);
var suppress={suppress_menu};
window.__ndSetSuppressMenu=function(v){{suppress=!!v;}};
document.addEventListener("contextmenu",function(e){{var el=e.target;
var editable=!!(el&&(el.isContentEditable||el.tagName==="INPUT"||el.tagName==="TEXTAREA"));
var image=(el&&el.tagName==="IMG"&&el.src)?el.src:"";
var sel=String(window.getSelection?window.getSelection():"");
post({{k:"menu",x:e.clientX,y:e.clientY,link:linkOf(el),image:image,selection:sel,editable:editable}});
if(suppress)e.preventDefault();}},true);
var muted=false;
var report=function(){{var m=document.querySelectorAll("video, audio");var p=false;
for(var i=0;i<m.length;i++)if(!m[i].paused&&!m[i].muted)p=true;post({{k:"audio",playing:p,muted:muted}});}};
window.__ndSetMuted=function(v){{muted=!!v;var m=document.querySelectorAll("video, audio");
for(var i=0;i<m.length;i++)m[i].muted=muted;report();}};
["play","pause","ended","volumechange"].forEach(function(n){{document.addEventListener(n,function(ev){{
if(muted&&ev.target&&"muted" in ev.target)ev.target.muted=true;report();}},true);}});
document.addEventListener("fullscreenchange",function(){{post({{k:"fullscreen",on:!!document.fullscreenElement}});}},true);
document.addEventListener("webkitfullscreenchange",function(){{post({{k:"fullscreen",on:!!document.webkitFullscreenElement}});}},true);
}})();"#
    )
}

/// Finds the page's best icon: the largest `<link rel=icon>`, else `/favicon.ico` for http(s).
/// Evaluates to a JSON string `{pageUrl, iconUrl}`.
pub const FAVICON_SCRIPT: &str = r#"(function(){var best="",size=-1;
var l=document.querySelectorAll("link[rel~='icon' i], link[rel='shortcut icon' i]");
for(var i=0;i<l.length;i++){var n=parseInt((l[i].getAttribute("sizes")||"").split("x")[0],10);if(isNaN(n))n=0;
if(n>=size){size=n;best=l[i].href;}}
if(!best&&location.protocol.indexOf("http")===0)best=location.origin+"/favicon.ico";
return JSON.stringify({pageUrl:location.href,iconUrl:best});})()"#;

/// Counts the pointer input the page receives into its title, as
/// `down=N up=N move=N wheel=N`, for the overlay leak gates (`nd_ui::InputCounts` parses it).
pub const COUNTING_SCRIPT: &str = nd_ui::surface::COUNTING_SCRIPT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_scripts_are_left_alone() {
        assert_eq!(UserScript::new("a", "x()").guarded_source(), "x()");
    }

    #[test]
    fn guarded_scripts_embed_both_lists() {
        let s = UserScript::new("a", "x()").allow(["https://*.example.com/*"]).block(["*/private/*"]).guarded_source();
        assert!(s.contains(r#"["https://*.example.com/*"]"#), "{s}");
        assert!(s.contains(r#"["*/private/*"]"#), "{s}");
        assert!(s.contains("\nx()\n"), "{s}");
    }
}
