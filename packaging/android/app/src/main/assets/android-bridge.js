// Loaded first by the bundled index.html (package.sh adds it). In a browser, PhotoCraft saves and
// exports by clicking a hidden <a download href="blob:...">; Android's WebView ignores those, so
// this hands the file to the app (MainActivity.Bridge), which writes it to Downloads/PhotoCraft.
(function () {
  "use strict";
  var bridge = window.PhotoCraftAndroid;
  if (!bridge) return;

  var CHUNK = 3 * 1024 * 1024; // a multiple of 3, so each chunk is whole base64

  function toBase64(bytes) {
    var s = "";
    for (var i = 0; i < bytes.length; i += 0x8000) {
      s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }
    return btoa(s);
  }

  async function save(url, name) {
    var id = -1;
    try {
      var blob = await (await fetch(url)).blob();
      id = bridge.begin(name, blob.type || "");
      if (id < 0) return; // the app has said why
      for (var off = 0; off < blob.size; off += CHUNK) {
        var part = new Uint8Array(await blob.slice(off, off + CHUNK).arrayBuffer());
        if (!bridge.append(id, toBase64(part))) return;
      }
      bridge.finish(id);
    } catch (e) {
      bridge.fail(id, String(e && e.message || e));
    }
  }

  var click = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    var href = this.href || "";
    if (this.hasAttribute("download") && href.indexOf("blob:") === 0) {
      // fetch() starts now, before the page revokes the blob URL.
      save(href, this.getAttribute("download") || "photocraft");
      return;
    }
    return click.call(this);
  };
})();
