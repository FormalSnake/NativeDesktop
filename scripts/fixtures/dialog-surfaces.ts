// The page the dialog-surface gates drive: one entry point per surface that
// Chromium draws itself rather than asking a CEF handler about. Shared by the
// GTK probe (examples/cef-probe) and the AppKit one
// (examples/webview-probe/cef-chrome.tsx) so both platforms fire the same
// requests.
//
// Every surface is a function on `window` and writes its outcome into
// `window.ndState`, so a drive can fire one at a time and read back whether the
// page's own promise ever settled.
//
// WebAuthn refuses an IP-address origin ("relying party ID is not a registrable
// domain"), so a probe serving this page has to reach it through a host name.
// `localhost` is the only one a gate with no network has.
export const DIALOG_SURFACES_PAGE: string =
  `<!doctype html><html><head><meta charset="utf-8"><title>ND CEF Dialogs</title></head>` +
  `<body style="font:16px sans-serif;background:#101014;color:#e8e8ef;margin:0"><div style="padding:24px">` +
  `<h1>dialog surfaces</h1>` +
  `<form id="login" method="post" action="/signed-in">` +
  `<input id="user" name="username" autocomplete="username webauthn" value="nd-user">` +
  `<input id="pass" name="password" type="password" autocomplete="current-password" value="nd-secret">` +
  `<button id="go" type="submit">sign in</button></form>` +
  `<a id="dl" href="/attachment" download="nd-gate.bin">download</a>` +
  `<iframe id="auth" width="200" height="60"></iframe>` +
  `<script>` +
  `window.ndState={};` +
  `function track(name,p){window.ndState[name]='pending';` +
  `Promise.resolve(p).then(v=>window.ndState[name]='ok:'+String(v).slice(0,40),` +
  `e=>window.ndState[name]='rejected:'+e.name);return name}` +
  `function challenge(){return new Uint8Array(32)}` +
  // One controller per passkey call, so a drive can end the request without a
  // key event: a Chromium sheet is a window of its own on AppKit and synthetic
  // input does not reach it, which would leave it on screen for every later leg.
  `var passkeyAbort=null;` +
  `function passkeySignal(){passkeyAbort=new AbortController();return passkeyAbort.signal}` +
  `window.ndAbortPasskey=()=>{if(passkeyAbort)passkeyAbort.abort();return 'aborted'};` +
  `window.ndPasskeyGet=()=>track('passkeyGet',navigator.credentials.get(` +
  `{signal:passkeySignal(),publicKey:{challenge:challenge(),timeout:60000,userVerification:'preferred'}}));` +
  `window.ndPasskeyConditional=()=>track('passkeyConditional',navigator.credentials.get(` +
  `{signal:passkeySignal(),mediation:'conditional',publicKey:{challenge:challenge(),timeout:60000}}));` +
  `window.ndPasskeyCreate=()=>track('passkeyCreate',navigator.credentials.create(` +
  `{signal:passkeySignal(),publicKey:{challenge:challenge(),rp:{name:'nd gate'},user:{id:challenge(),name:'nd',displayName:'nd'},` +
  `pubKeyCredParams:[{type:'public-key',alg:-7}],timeout:60000}}));` +
  `window.ndGeolocation=()=>track('geolocation',new Promise((res,rej)=>` +
  `navigator.geolocation.getCurrentPosition(()=>res('position'),e=>rej(new Error(e.message||'denied')))));` +
  `window.ndNotifications=()=>track('notifications',Notification.requestPermission());` +
  `window.ndCamera=()=>track('camera',navigator.mediaDevices.getUserMedia({video:true}));` +
  `window.ndAlert=()=>{setTimeout(()=>alert('nd gate alert'),0);return 'alert'};` +
  `window.ndConfirm=()=>{setTimeout(()=>{window.ndState.confirm='ok:'+confirm('nd gate confirm')},0);return 'confirm'};` +
  `window.ndPrompt=()=>{setTimeout(()=>{window.ndState.prompt='ok:'+prompt('nd gate prompt','x')},0);return 'prompt'};` +
  `window.ndHttpAuth=()=>{document.getElementById('auth').src='/auth-basic';return 'auth'};` +
  `window.ndDownload=()=>{document.getElementById('dl').click();return 'download'};` +
  `window.ndPasswordSubmit=()=>{document.getElementById('login').submit();return 'submit'};` +
  `</script></div></body></html>`;

/// The routes the page above needs from whatever server hosts it: a 401 to
/// bring up HTTP auth, an attachment to start a download, and the page the
/// login form posts to.
export function dialogSurfacesRoute(path: string): Response | null {
  if (path === "/dialogs") {
    return new Response(DIALOG_SURFACES_PAGE, { headers: { "content-type": "text/html; charset=utf-8" } });
  }
  if (path === "/auth-basic") {
    return new Response("denied", {
      status: 401,
      headers: { "www-authenticate": 'Basic realm="nd-gate"', "content-type": "text/plain" },
    });
  }
  if (path === "/attachment") {
    return new Response("nd-gate-attachment", {
      headers: {
        "content-type": "application/octet-stream",
        "content-disposition": 'attachment; filename="nd-gate.bin"',
      },
    });
  }
  if (path === "/signed-in") {
    return new Response("<!doctype html><title>ND CEF Signed In</title><h1>signed in</h1>", {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }
  return null;
}
