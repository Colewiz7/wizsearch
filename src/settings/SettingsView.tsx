import { openUrl } from "@tauri-apps/plugin-opener";
import { useEffect, useMemo, useState } from "react";
import {
  secretClear,
  secretExists,
  secretSet,
  sidecarsEnsure,
  sidecarsStatus,
  ytdlpUpdate,
} from "../api/commands";
import type { SettingDef, SidecarStatus } from "../api/types";
import { useSettings } from "./useSettings";

/** the whole panel is generated from the registry; no hand-built controls */
export function SettingsView() {
  const { defs, values, setValue } = useSettings();
  const [errors, setErrors] = useState<Record<string, string>>({});

  const categories = useMemo(() => {
    const cats: Record<string, SettingDef[]> = {};
    for (const d of defs) {
      (cats[d.category] ??= []).push(d);
    }
    return cats;
  }, [defs]);

  const onChange = async (key: string, value: unknown) => {
    try {
      await setValue(key, value);
      setErrors((e) => ({ ...e, [key]: "" }));
    } catch (err) {
      setErrors((e) => ({ ...e, [key]: String(err) }));
    }
  };

  return (
    <div className="settings-view">
      {Object.entries(categories).map(([cat, catDefs]) => (
        <section key={cat} className="settings-section">
          <h2>{cat}</h2>
          {catDefs.map((def) => (
            <div key={def.key} className="setting-row">
              <div className="setting-text">
                <label htmlFor={def.key}>{def.label}</label>
                <p>{def.description}</p>
                {errors[def.key] && <p className="setting-error">{errors[def.key]}</p>}
              </div>
              <div className="setting-control">
                <SettingControl
                  def={def}
                  value={values[def.key]}
                  onChange={(v) => void onChange(def.key, v)}
                />
              </div>
            </div>
          ))}
        </section>
      ))}
      <SidecarPanel />
    </div>
  );
}

function SettingControl({
  def,
  value,
  onChange,
}: {
  def: SettingDef;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const k = def.kind;
  switch (k.type) {
    case "bool":
      return (
        <input
          id={def.key}
          type="checkbox"
          checked={Boolean(value ?? k.default)}
          onChange={(e) => onChange(e.target.checked)}
        />
      );
    case "int":
      return (
        <input
          id={def.key}
          type="number"
          min={k.min}
          max={k.max}
          step={1}
          defaultValue={Number(value ?? k.default)}
          onBlur={(e) => onChange(Math.round(Number(e.target.value)))}
        />
      );
    case "float":
      return (
        <input
          id={def.key}
          type="range"
          min={k.min}
          max={k.max}
          step={0.05}
          defaultValue={Number(value ?? k.default)}
          onChange={(e) => onChange(Number(e.target.value))}
        />
      );
    case "text":
      return (
        <input
          id={def.key}
          type="text"
          placeholder={k.placeholder}
          defaultValue={String(value ?? k.default)}
          onBlur={(e) => onChange(e.target.value)}
        />
      );
    case "select":
      return (
        <select
          id={def.key}
          value={String(value ?? k.default)}
          onChange={(e) => onChange(e.target.value)}
        >
          {k.options.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      );
    case "secret":
      return <SecretField settingKey={def.key} helpUrl={k.help_url} />;
  }
}

/** write-only keychain field: the UI can set/clear but never read the value */
function SecretField({ settingKey, helpUrl }: { settingKey: string; helpUrl: string }) {
  const [isSet, setIsSet] = useState<boolean | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    secretExists(settingKey).then(setIsSet).catch((e) => setError(String(e)));
  }, [settingKey]);

  const save = async () => {
    if (!draft.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await secretSet(settingKey, draft.trim());
      setIsSet(true);
      setDraft("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      await secretClear(settingKey);
      setIsSet(false);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="secret-field">
      <span className={`secret-dot ${isSet ? "secret-set" : ""}`}>
        {isSet === null ? "…" : isSet ? "set" : "not set"}
      </span>
      <input
        type="password"
        placeholder={isSet ? "replace key" : "paste key"}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && void save()}
      />
      <button className="mini" onClick={save} disabled={busy || !draft.trim()}>
        Save
      </button>
      {isSet && (
        <button className="mini danger" onClick={clear} disabled={busy}>
          Clear
        </button>
      )}
      {helpUrl && (
        <a
          href={helpUrl}
          className="key-help"
          onClick={(e) => {
            // webviews don't follow target=_blank; open the system browser
            e.preventDefault();
            void openUrl(helpUrl);
          }}
        >
          get a key
        </a>
      )}
      {error && <span className="setting-error">{error}</span>}
    </div>
  );
}

function SidecarPanel() {
  const [statuses, setStatuses] = useState<SidecarStatus[]>([]);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const refresh = () => sidecarsStatus().then(setStatuses).catch((e) => setMessage(String(e)));
  useEffect(() => {
    void refresh();
  }, []);

  const ensure = async () => {
    setBusy(true);
    setMessage("Downloading and verifying (SHA-256)…");
    try {
      setStatuses(await sidecarsEnsure());
      setMessage(null);
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };

  const updateYtdlp = async () => {
    setBusy(true);
    try {
      setMessage(await ytdlpUpdate());
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="settings-section">
      <h2>Bundled tools</h2>
      <div className="sidecar-list">
        {statuses.map((s) => (
          <div key={s.name} className="sidecar-row">
            <code>{s.name}</code>
            <span className={s.installed ? "ok" : "missing"}>
              {s.installed ? "installed" : "missing"}
            </span>
            {s.pinned ? <span className="ok">pinned</span> : <span className="missing">unpinned</span>}
          </div>
        ))}
      </div>
      <div className="action-row">
        <button className="mini" onClick={ensure} disabled={busy}>
          Download missing tools
        </button>
        <button className="mini" onClick={updateYtdlp} disabled={busy}>
          Update yt-dlp
        </button>
      </div>
      {message && <p className="hint-row">{message}</p>}
    </section>
  );
}
