(() => {
  "use strict";
  const storageKey = "exaudio.site.language";
  const releases = "https://github.com/Nominy/exaudio/releases/latest/download/";

  // Per-OS download offer. Applied before the language cache below is built, so both
  // languages of the main button, the alternative button and the note follow the visitor's OS.
  const platform = navigator.userAgentData?.platform ?? navigator.platform ?? "";
  const os = /Win/i.test(platform) ? "win"
    : /Mac/i.test(platform) ? "mac"
    : /Linux|X11/i.test(platform) && !/Android/i.test(navigator.userAgent) ? "linux"
    : null;
  // Offers fall back to the page's default (Windows) when the OS is unknown.
  const offers = {
    win: {
      main: { file: "Exaudio-win-Setup.exe", ru: "Скачать для Windows", en: "Download for Windows" },
      alt: { file: "Exaudio-win-Portable.zip", ru: "Portable .zip", en: "Portable .zip" },
      note: {
        ru: "Windows 10 и 11, x64. Если SmartScreen пишет «Система Windows защитила ваш компьютер», нажмите <b>Подробнее → Выполнить в любом случае</b>: Exaudio пока не подписан.",
        en: "Windows 10 and 11, x64. If SmartScreen says “Windows protected your PC”, click <b>More info → Run anyway</b>: Exaudio isn't code-signed yet.",
      },
      portable: {
        cmd: "irm " + releases + "Exaudio-win-Portable.zip -OutFile Exaudio.zip; Expand-Archive Exaudio.zip Exaudio; .\\Exaudio\\Exaudio.exe",
        ru: "PowerShell. Распаковывает рядом и запускает редактор; настройки и снимки восстановления — в %APPDATA%\\Exaudio. Обновления предлагаются внутри приложения.",
        en: "PowerShell. Unpacks next to you and starts the editor; settings and recovery snapshots live in %APPDATA%\\Exaudio. Updates are offered inside the app.",
      },
    },
    mac: {
      main: { file: "Exaudio-mac-arm64.dmg", ru: "Скачать для Mac", en: "Download for Mac" },
      alt: { file: "Exaudio-mac-x64.dmg", ru: "Для Intel Mac", en: "Intel Mac" },
      note: {
        ru: "macOS 14 и новее. Откройте образ и перетащите Exaudio в «Программы». При первом запуске macOS не сможет проверить приложение (оно пока не нотаризовано): откройте <b>Системные настройки → Конфиденциальность и безопасность</b> и нажмите <b>Всё равно открыть</b>.",
        en: "macOS 14 or later. Open the disk image and drag Exaudio to Applications. The first time, macOS can't verify it (it isn't notarized yet): open <b>System Settings → Privacy &amp; Security</b> and click <b>Open Anyway</b>.",
      },
      portable: {
        cmd: "curl -fsSLO " + releases + "Exaudio-mac-arm64.dmg && open Exaudio-mac-arm64.dmg",
        ru: "Скачивает образ и открывает его; перетащите Exaudio в «Программы». Настройки и снимки восстановления — в ~/Library/Application Support/Exaudio. Обновления предлагаются внутри приложения.",
        en: "Downloads and opens the disk image; drag Exaudio to Applications. Settings and recovery snapshots live in ~/Library/Application Support/Exaudio. Updates are offered inside the app.",
      },
    },
    linux: {
      main: { file: "Exaudio-linux-x64.AppImage", ru: "Скачать для Linux", en: "Download for Linux" },
      alt: { file: "Exaudio-linux-arm64.AppImage", ru: "AppImage для arm64", en: "arm64 AppImage" },
      note: {
        ru: "64-битный Linux с X11 или Wayland. Сделайте файл исполняемым (<code>chmod +x</code>) и запустите; в большинстве дистрибутивов нужен FUSE 2 (<code>libfuse2</code>). Локальный ИИ работает только в Windows — используйте удалённый сервер.",
        en: "64-bit Linux with X11 or Wayland. Make the file executable (<code>chmod +x</code>) and run it; most distributions need FUSE 2 (<code>libfuse2</code>). Local AI runs on Windows only — use the remote server.",
      },
      portable: {
        cmd: "curl -fsSLO " + releases + "Exaudio-linux-x64.AppImage && chmod +x Exaudio-linux-x64.AppImage && ./Exaudio-linux-x64.AppImage",
        ru: "Скачивает AppImage рядом и запускает редактор; настройки и снимки восстановления — в ~/.config/Exaudio. Обновления предлагаются внутри приложения.",
        en: "Downloads the AppImage next to you and starts the editor; settings and recovery snapshots live in ~/.config/Exaudio. Updates are offered inside the app.",
      },
    },
  };
  const offer = offers[os ?? "win"];
  function setText(id, text) {
    const node = document.getElementById(id);
    node.innerHTML = text.ru;
    node.dataset.en = text.en;
  }
  function applyOffer(current) {
    document.getElementById("download").dataset.os = os ?? "win";
    document.getElementById("dl-main").href = releases + current.main.file;
    document.getElementById("dl-alt").href = releases + current.alt.file;
    setText("dl-main-text", current.main);
    setText("dl-alt", current.alt);
    setText("dl-note", current.note);
    setText("hint-portable", current.portable);
  }
  applyOffer(offer);
  // Chromium on an Intel Mac can tell us; make that the main button. Runs after the language
  // cache below exists, so the swapped nodes are re-cached and re-rendered.
  if (os === "mac") navigator.userAgentData?.getHighEntropyValues(["architecture"]).then(values => {
    if (values.architecture !== "x86") return;
    const intel = {
      ...offer,
      main: { ...offer.main, file: "Exaudio-mac-x64.dmg" },
      alt: { file: "Exaudio-mac-arm64.dmg", ru: "Для Apple silicon", en: "Apple silicon" },
      portable: { ...offer.portable, cmd: offer.portable.cmd.replaceAll("Exaudio-mac-arm64.dmg", "Exaudio-mac-x64.dmg") },
    };
    applyOffer(intel);
    for (const id of ["dl-main-text", "dl-alt", "dl-note", "hint-portable"]) refreshTextNode(document.getElementById(id));
    commands.portable = intel.portable.cmd;
    if (document.getElementById("tab-portable").getAttribute("aria-selected") === "true") cmdText.textContent = commands.portable;
    setLanguage(document.documentElement.lang);
  }).catch(() => { /* Older browsers lack high-entropy hints; the Apple silicon offer stays. */ });

  const textNodes = Array.from(document.querySelectorAll("[data-en]"), node => ({
    node,
    russian: Array.from(node.childNodes, child => child.cloneNode(true)),
    english: node.dataset.en,
  }));
  function refreshTextNode(node) {
    const item = textNodes.find(entry => entry.node === node);
    item.russian = Array.from(node.childNodes, child => child.cloneNode(true));
    item.english = node.dataset.en;
  }
  const labels = Array.from(document.querySelectorAll("[data-en-label]"), node => ({
    node,
    russian: node.getAttribute("aria-label"),
    english: node.dataset.enLabel,
  }));
  const alts = Array.from(document.querySelectorAll("img[data-en-alt]"), node => ({
    node,
    russian: node.getAttribute("alt"),
    english: node.dataset.enAlt,
  }));
  const buttons = document.querySelectorAll("[data-language]");
  const descriptions = {
    ru: "Exaudio — сегментный редактор транскрипций с аудио. Локальные проекты без аккаунта, расширения и необязательный ИИ. Для Windows, macOS и Linux.",
    en: "Exaudio is a segment-based transcription editor with audio. Local projects without an account, extensions and optional AI. For Windows, macOS and Linux.",
  };

  function setLanguage(language) {
    const english = language === "en";
    document.documentElement.lang = language;
    for (const item of textNodes) {
      // English strings are authored in this file and may carry inline markup (<span>, <b>, <a>).
      if (english) item.node.innerHTML = item.english;
      else item.node.replaceChildren(...item.russian.map(child => child.cloneNode(true)));
    }
    for (const item of labels) {
      item.node.setAttribute("aria-label", english ? item.english : item.russian);
    }
    for (const item of alts) {
      item.node.setAttribute("alt", english ? item.english : item.russian);
    }
    for (const button of buttons) {
      button.setAttribute("aria-pressed", String(button.dataset.language === language));
    }
    document.title = english
      ? "Exaudio — a transcription editor with audio"
      : "Exaudio — редактор транскрипций с аудио";
    document.querySelector('meta[name="description"]').content = descriptions[language];
    document.querySelector('meta[property="og:title"]').content = document.title;
    document.querySelector('meta[property="og:description"]').content = descriptions[language];
    try { localStorage.setItem(storageKey, language); } catch { /* Private browsing may deny storage. */ }
  }

  for (const button of buttons) {
    button.addEventListener("click", () => setLanguage(button.dataset.language));
  }

  const commands = {
    portable: offer.portable.cmd,
    source: "git clone https://github.com/Nominy/exaudio.git; cd exaudio; cargo run -p studio-app",
  };
  const cmdText = document.getElementById("cmd-text");
  const tabs = Array.from(document.querySelectorAll('[role="tab"]'));
  function selectTab(tab) {
    for (const other of tabs) {
      const selected = other === tab;
      other.setAttribute("aria-selected", String(selected));
      other.tabIndex = selected ? 0 : -1;
      document.getElementById("hint-" + other.dataset.cmd).hidden = !selected;
    }
    cmdText.textContent = commands[tab.dataset.cmd];
  }
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => selectTab(tab));
    tab.addEventListener("keydown", event => {
      const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
      if (!step) return;
      event.preventDefault();
      const next = tabs[(index + step + tabs.length) % tabs.length];
      selectTab(next);
      next.focus();
    });
  });

  const copy = document.getElementById("copy");
  copy.addEventListener("click", async () => {
    const label = copy.textContent;
    try {
      await navigator.clipboard.writeText(cmdText.textContent);
      copy.textContent = document.documentElement.lang === "en" ? "Copied" : "Скопировано";
      setTimeout(() => { copy.textContent = label; }, 1500);
    } catch { /* Clipboard access denied; the command stays selectable. */ }
  });

  let language = "ru";
  try { if (localStorage.getItem(storageKey) === "en") language = "en"; } catch { /* Russian remains the default. */ }
  setLanguage(language);
})();
