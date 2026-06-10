import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { settingsDefs, settingsSet, settingsValues } from "../api/commands";
import { initStream } from "../api/stream";
import type { SettingDef } from "../api/types";

interface SettingsCtx {
  defs: SettingDef[];
  values: Record<string, unknown>;
  setValue: (key: string, value: unknown) => Promise<void>;
  ready: boolean;
}

const Ctx = createContext<SettingsCtx>({
  defs: [],
  values: {},
  setValue: async () => {},
  ready: false,
});

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [defs, setDefs] = useState<SettingDef[]>([]);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [ready, setReady] = useState(false);

  useEffect(() => {
    Promise.all([settingsDefs(), settingsValues(), initStream()])
      .then(([d, v]) => {
        setDefs(d);
        setValues(v);
        setReady(true);
      })
      .catch((e) => console.error("settings load failed", e));
  }, []);

  const setValue = useCallback(async (key: string, value: unknown) => {
    await settingsSet(key, value);
    setValues((prev) => ({ ...prev, [key]: value }));
  }, []);

  // hold rendering until stream urls can be built
  return (
    <Ctx.Provider value={{ defs, values, setValue, ready }}>
      {ready ? children : null}
    </Ctx.Provider>
  );
}

export function useSettings() {
  return useContext(Ctx);
}

export function useSetting<T>(key: string, fallback: T): T {
  const { values } = useSettings();
  const v = values[key];
  return (v === undefined || v === null ? fallback : v) as T;
}
