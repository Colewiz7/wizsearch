import { useEffect, useState } from "react";
import { CollectionView } from "./collection/CollectionView";
import { SearchView } from "./grid/SearchView";
import { SettingsView } from "./settings/SettingsView";
import { SettingsProvider, useSetting } from "./settings/useSettings";

type Tab = "search" | "library" | "settings";

function Shell() {
  const [tab, setTab] = useState<Tab>("search");
  const theme = useSetting("ui.theme", "dark");

  useEffect(() => {
    document.documentElement.dataset.theme = String(theme);
  }, [theme]);

  return (
    <div className="app">
      <nav className="topnav">
        <span className="brand">WizSearch</span>
        {(["search", "library", "settings"] as Tab[]).map((t) => (
          <button
            key={t}
            className={`nav-tab ${tab === t ? "nav-active" : ""}`}
            onClick={() => setTab(t)}
          >
            {t[0].toUpperCase() + t.slice(1)}
          </button>
        ))}
      </nav>
      <main className="content">
        {/* search stays mounted so results survive tab switches */}
        <div style={{ display: tab === "search" ? "contents" : "none" }}>
          <SearchView />
        </div>
        {tab === "library" && <CollectionView />}
        {tab === "settings" && <SettingsView />}
      </main>
    </div>
  );
}

export default function App() {
  return (
    <SettingsProvider>
      <Shell />
    </SettingsProvider>
  );
}
