// Pincers browser integration — captures downloads like the "IDM Integration
// Module": when the browser starts a download of a configured file type, the
// extension cancels it and shows the link in a small window that copies it
// to the clipboard, ready to paste into Pincers.
//
// The same file runs in Chrome/Opera (Manifest V3 service worker) and in
// Firefox (Manifest V2 background script).

// Firefox: `browser` (promises). Chrome/Opera: `chrome` (recent Chromium also
// exposes a partial `browser`, so test for Firefox explicitly).
const IS_FIREFOX = typeof browser !== "undefined" && typeof browser.runtime?.getBrowserInfo === "function";
const api = IS_FIREFOX ? browser : chrome;
const toolbar = api.action ?? api.browserAction;

// IDM's default "Automatically start downloading the following file types".
const DEFAULT_EXTENSIONS = [
  "3gp", "7z", "aac", "ace", "aif", "apk", "arj", "asf", "avi", "bin", "bz2", "dmg", "exe", "gz", "gzip",
  "img", "iso", "lzh", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "mpa", "mpe", "mpeg", "mpg", "msi", "msix",
  "msu", "ogg", "ogv", "pdf", "plj", "pps", "ppt", "qt", "r0*", "r1*", "ra", "rar", "rm", "rmvb", "sea", "sit",
  "sitx", "tar", "tif", "tiff", "wav", "webm", "wma", "wmv", "xz", "z", "zip", "zst",
];

const DEFAULTS = {
  enabled: true,
  extensions: DEFAULT_EXTENSIONS.join(" "),
  // Hosts whose downloads always stay in the browser, one per line.
  excluded: "",
};

// URLs the user chose to download in the browser (Alt+click or the
// "Baixar no navegador" button): the next download of them is not captured.
const BYPASS_KEY = "bypass";
// Downloads waiting for the browser to decide their file name.
const pending = new Map();

// ------------------------------------------------------------ diagnostics

/** Last events, kept in storage: a service worker's console vanishes
 * whenever the browser stops it. Shown at the bottom of the popup. */
async function log(...parts) {
  const line = `${new Date().toLocaleTimeString()} ${parts.join(" ")}`;
  const { debugLog = [] } = await api.storage.local.get("debugLog");
  debugLog.push(line.slice(0, 300));
  await api.storage.local.set({ debugLog: debugLog.slice(-30) });
}

// ------------------------------------------------------------ settings

async function settings() {
  return { ...DEFAULTS, ...(await api.storage.local.get(Object.keys(DEFAULTS))) };
}

function extensionPatterns(list) {
  return list
    .split(/[\s,;]+/)
    .map((e) => e.trim().replace(/^\./, "").toLowerCase())
    .filter(Boolean)
    .map((e) => new RegExp("^" + e.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*") + "$"));
}

function isExcluded(url, excluded) {
  let host;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return true;
  }
  return excluded
    .split(/\s+/)
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean)
    .some((h) => host === h || host.endsWith("." + h));
}

// ------------------------------------------------------------ file names

function baseName(path) {
  return (path || "").split(/[\\/]/).pop() || "";
}

function nameFromUrl(url) {
  try {
    return decodeURIComponent(baseName(new URL(url).pathname));
  } catch {
    return "";
  }
}

function extOf(name) {
  const m = /\.([^.]+)$/.exec(name || "");
  return m ? m[1].toLowerCase() : "";
}

// For downloads whose URL carries no file name (e.g. signed CDN links).
const MIME_EXTENSIONS = {
  "application/zip": "zip", "application/x-zip-compressed": "zip", "application/x-7z-compressed": "7z",
  "application/x-rar-compressed": "rar", "application/vnd.rar": "rar", "application/gzip": "gz",
  "application/x-gzip": "gz", "application/x-tar": "tar", "application/x-xz": "xz", "application/pdf": "pdf",
  "application/x-msdownload": "exe", "application/x-msdos-program": "exe", "application/x-msi": "msi",
  "application/x-iso9660-image": "iso", "application/vnd.android.package-archive": "apk",
  "video/mp4": "mp4", "video/x-matroska": "mkv", "video/webm": "webm", "video/quicktime": "mov",
  "video/x-msvideo": "avi", "audio/mpeg": "mp3", "audio/mp4": "m4a", "audio/wav": "wav", "audio/ogg": "ogg",
};

/** `filename=` that CDNs embed in signed links (`response-content-disposition=…`). */
function nameFromLinkQuery(url) {
  let text;
  try {
    text = decodeURIComponent(url || "");
  } catch {
    text = url || "";
  }
  const m = /filename\*?=(?:UTF-8'')?"?([^";&]+)/i.exec(text);
  return m ? baseName(m[1].trim()) : "";
}

/** Best file name for a browser download, from the most to the least reliable. */
function guessName(item) {
  const candidates = [
    baseName(item.filename),
    nameFromUrl(item.url),
    nameFromLinkQuery(item.finalUrl),
    nameFromUrl(item.finalUrl),
  ];
  const named = candidates.find((c) => extOf(c));
  if (named) return named;
  const ext = MIME_EXTENSIONS[(item.mime || "").split(";")[0].trim().toLowerCase()];
  return ext ? `${candidates.find(Boolean) || "download"}.${ext}` : "";
}

function capturable(url) {
  return /^(https?|ftp):/i.test(url || "");
}

// ------------------------------------------------------- captured window

/** Remembers the links and opens the window that copies them. */
async function showCaptured(links, source) {
  const { captured = [] } = await api.storage.local.get("captured");
  const entry = { id: Date.now(), links, source, time: Date.now() };
  captured.unshift(entry);
  await api.storage.local.set({ captured: captured.slice(0, 30) });
  await api.windows.create({
    url: api.runtime.getURL(`captured.html?id=${entry.id}`),
    type: "popup",
    width: 520,
    height: links.length > 1 ? 420 : 300,
    focused: true,
  });
  toolbar.setBadgeBackgroundColor?.({ color: "#f0643a" });
  toolbar.setBadgeText({ text: String(links.length) });
  setTimeout(() => toolbar.setBadgeText({ text: "" }), 3000);
}

// ------------------------------------------------------ automatic capture

api.downloads.onCreated.addListener(async (item) => {
  // The URL that was clicked: redirect targets are often signed links that
  // expire, while the original redirects again when Pincers resumes.
  const url = capturable(item.url) ? item.url : item.finalUrl;
  if (item.state && item.state !== "in_progress") return;
  if (!capturable(url)) return;
  const { [BYPASS_KEY]: bypass = {} } = await api.storage.local.get(BYPASS_KEY);
  if (bypass.until > Date.now() && (!bypass.url || bypass.url === url || bypass.url === item.finalUrl)) {
    await api.storage.local.remove(BYPASS_KEY);
    return log("deixado com o navegador:", url.slice(0, 100));
  }
  const s = await settings();
  if (!s.enabled || isExcluded(url, s.excluded)) return;

  const name = guessName(item);
  if (extOf(name)) {
    decide(item, url, name, s);
  } else {
    // Chrome sometimes names the file only after onCreated: wait a moment.
    pending.set(item.id, { item, url, s, timer: setTimeout(() => decide(item, url, guessName(item), s), 1500) });
  }
});

api.downloads.onChanged.addListener((delta) => {
  const p = pending.get(delta.id);
  if (p && delta.filename?.current) {
    clearTimeout(p.timer);
    decide(p.item, p.url, baseName(delta.filename.current), p.s);
  }
});

async function decide(item, url, name, s) {
  pending.delete(item.id);
  const ext = extOf(name);
  if (!ext || !extensionPatterns(s.extensions).some((re) => re.test(ext))) {
    return log("não capturado (tipo", JSON.stringify(ext || "?") + "):", name || url.slice(0, 80));
  }
  await api.downloads.cancel(item.id).catch(() => {});
  await api.downloads.erase({ id: item.id }).catch(() => {});
  log("capturado:", name);
  await showCaptured([{ url, name, referrer: item.referrer || "" }], "download");
}

// ---------------------------------------------------------- context menus

function createMenus() {
  // Promise form: Firefox's `browser.*` APIs take no callbacks.
  Promise.resolve(api.contextMenus.removeAll()).then(() => {
    api.contextMenus.create({
      id: "pincers-link",
      title: "Copiar link para o Pincers",
      contexts: ["link", "image", "video", "audio"],
    });
    api.contextMenus.create({
      id: "pincers-all",
      title: "Copiar todos os links de arquivos para o Pincers",
      contexts: ["page", "selection"],
    });
  });
}

api.runtime.onInstalled.addListener(createMenus);
api.runtime.onStartup?.addListener(createMenus);

api.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "pincers-link") {
    const url = info.linkUrl || info.srcUrl;
    if (capturable(url)) await showCaptured([{ url, name: nameFromUrl(url), referrer: info.pageUrl }], "menu");
  } else if (info.menuItemId === "pincers-all" && tab?.id != null) {
    const s = await settings();
    const patterns = extensionPatterns(s.extensions);
    // Like IDM's "Download all links": only files of the configured types.
    const wanted = [...new Set(await collectLinks(tab.id))]
      .filter((u) => capturable(u) && patterns.some((re) => re.test(extOf(nameFromUrl(u)))))
      .slice(0, 500);
    if (wanted.length) {
      await showCaptured(wanted.map((url) => ({ url, name: nameFromUrl(url), referrer: info.pageUrl })), "page");
    } else {
      toolbar.setBadgeBackgroundColor?.({ color: "#e8b04a" });
      toolbar.setBadgeText({ text: "0" });
      setTimeout(() => toolbar.setBadgeText({ text: "" }), 2000);
    }
  }
});

/** Links in the current selection, or in the whole page without one. */
async function collectLinks(tabId) {
  const grab = () => {
    const sel = window.getSelection();
    let anchors = [...document.querySelectorAll("a[href]")];
    if (sel && !sel.isCollapsed) anchors = anchors.filter((a) => sel.containsNode(a, true));
    return anchors.map((a) => a.href);
  };
  try {
    if (api.scripting) {
      const res = await api.scripting.executeScript({ target: { tabId, allFrames: true }, func: grab });
      return res.flatMap((r) => r.result ?? []);
    }
    const res = await api.tabs.executeScript(tabId, { code: `(${grab})()`, allFrames: true });
    return res.flat();
  } catch {
    return [];
  }
}

// ------------------------------------------------------------- messages

api.runtime.onMessage.addListener((msg) => {
  if (msg?.type === "bypass") {
    // Alt+click on a link (content.js) or "Baixar no navegador": the next
    // download (of that URL, when given) goes to the browser.
    api.storage.local.set({ [BYPASS_KEY]: { url: msg.url || null, until: Date.now() + 5000 } });
  }
});
