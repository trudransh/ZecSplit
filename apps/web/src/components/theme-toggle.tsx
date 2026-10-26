"use client";

export function ThemeToggle() {
  const flip = () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try {
      localStorage.setItem("theme", next);
    } catch {}
  };

  return (
    <button type="button" className="theme-toggle" onClick={flip} aria-label="Switch light or dark theme">
      <svg viewBox="0 0 20 20" width="18" height="18" aria-hidden="true">
        <circle cx="10" cy="10" r="7.5" fill="none" stroke="currentColor" strokeWidth="1.5" />
        <path d="M10 2.5a7.5 7.5 0 0 1 0 15z" fill="currentColor" />
      </svg>
    </button>
  );
}
