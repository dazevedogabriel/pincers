const IS_FIREFOX = typeof browser !== "undefined" && typeof browser.runtime?.getBrowserInfo === "function";
const api = IS_FIREFOX ? browser : chrome;
const $ = (id) => document.getElementById(id);

/** Copies `text`; extension pages may use the async clipboard while focused,
 * with the older execCommand as a fallback. */
async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    return ok;
  }
}

function badge(name) {
  const m = /\.([^.]+)$/.exec(name || "");
  return m ? m[1].slice(0, 4).toUpperCase() : "URL";
}

async function main() {
  const id = Number(new URLSearchParams(location.search).get("id"));
  const { captured = [] } = await api.storage.local.get("captured");
  const entry = captured.find((e) => e.id === id) ?? captured[0];
  if (!entry) return;
  const links = entry.links;
  // One URL per line: Pincers' URL field accepts several at once.
  const all = links.map((l) => l.url).join("\n");

  const many = links.length > 1;
  $("title").textContent = many ? `${links.length} links capturados` : entry.source === "download" ? "Download capturado" : "Link copiado";
  $("sub").textContent = links[0].name && !many ? links[0].name : "Cole no Pincers para baixar";
  $("copy-all").textContent = many ? "Copiar todos" : "Copiar";
  $("copied-text").textContent = many ? `${links.length} links copiados — cole no Pincers (Ctrl+V)` : "Link copiado — cole no Pincers (Ctrl+V)";
  $("browser").hidden = many || entry.source !== "download";

  for (const l of links) {
    const li = document.createElement("li");
    li.innerHTML = `<span class="badge"></span><span class="text"><span class="name"></span><span class="url"></span></span><button>Copiar</button>`;
    li.querySelector(".badge").textContent = badge(l.name);
    li.querySelector(".name").textContent = l.name || l.url;
    li.querySelector(".url").textContent = l.url;
    li.querySelector(".url").title = l.url;
    li.querySelector("button").addEventListener("click", async (e) => {
      if (await copy(l.url)) e.target.textContent = "Copiado ✓";
    });
    $("links").append(li);
  }

  const done = await copy(all);
  $("copied").hidden = !done;
  $("copy-failed").hidden = done;

  $("copy-all").addEventListener("click", async () => {
    const ok = await copy(all);
    $("copied").hidden = !ok;
    $("copy-failed").hidden = ok;
  });
  $("browser").addEventListener("click", async () => {
    // Let the browser download it this time, without capturing it again.
    await api.runtime.sendMessage({ type: "bypass", url: links[0].url });
    await api.downloads.download({ url: links[0].url });
    window.close();
  });
  $("close").addEventListener("click", () => window.close());
}

main();
