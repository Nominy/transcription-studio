(() => {
  "use strict";
  const storageKey = "sonus.site.language";
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
  const buttons = document.querySelectorAll("[data-language]");
  const descriptions = {
    ru: "Exaudio — сегментный редактор транскрипций с аудио. Локальные проекты без аккаунта, расширения и необязательный ИИ. Для Windows.",
    en: "Exaudio is a segment-based transcription editor with audio. Local projects without an account, extensions and optional AI. For Windows.",
  };

  function setLanguage(language) {
    const english = language === "en";
    document.documentElement.lang = language;
    for (const item of textNodes) {
      if (english) item.node.textContent = item.english;
      else item.node.replaceChildren(...item.russian.map(child => child.cloneNode(true)));
    }
    for (const item of labels) {
      item.node.setAttribute("aria-label", english ? item.english : item.russian);
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
  let language = "ru";
  try { if (localStorage.getItem(storageKey) === "en") language = "en"; } catch { /* Russian remains the default. */ }
  setLanguage(language);
})();
