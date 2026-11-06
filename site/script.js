(() => {
  "use strict";
  const storageKey = "exaudio.site.language";
  const textNodes = Array.from(document.querySelectorAll("[data-en]"), node => ({
    node,
    russian: Array.from(node.childNodes, child => child.cloneNode(true)),
    english: node.dataset.en,
  }));
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
    ru: "Exaudio — сегментный редактор транскрипций с аудио. Локальные проекты без аккаунта, расширения и необязательный ИИ. Для Windows.",
    en: "Exaudio is a segment-based transcription editor with audio. Local projects without an account, extensions and optional AI. For Windows.",
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
    portable: "irm https://github.com/Nominy/exaudio/releases/latest/download/Exaudio-win-Portable.zip -OutFile Exaudio.zip; Expand-Archive Exaudio.zip Exaudio; .\\Exaudio\\Exaudio.exe",
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
