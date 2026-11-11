import { useState } from "react";
import type { GogglesMode, Settings, Status } from "../types";
import { PanelHead } from "./PanelHead";
import { IconGoggles } from "./icons";

const CHOICES: { value: GogglesMode; label: string }[] = [
  { value: "auto", label: "Automatic (recommended)" },
  { value: "goggles3", label: "DJI Goggles 3 (O3 / O4 air unit)" },
  { value: "goggles2", label: "DJI Goggles 2 (experimental)" },
  { value: "v1v2", label: "DJI FPV Goggles V1 / V2 (experimental)" },
];
const SYSTEM_NAMES: Record<string, string> = {
  goggles3: "Goggles 3",
  goggles2: "Goggles 2",
  v1v2: "FPV Goggles V1/V2",
};

/**
 * Which goggles are plugged in. Automatic finds out by itself (and remembers
 * it for those goggles); a fixed choice never tries anything else, which is
 * what to pick when automatic does not find a pair of goggles, or to keep a
 * working setup from ever being probed with another generation's requests.
 * Changing it restarts the receiver, so the goggles connect again.
 */
export function GogglesPanel({
  settings,
  status,
  detected,
  disabled,
  onSave,
}: {
  settings: Settings | null;
  status: Status | null;
  /** What the receiver reports right now, e.g. "Goggles 3 (detected)". */
  detected: string;
  disabled: boolean;
  onSave: (patch: Partial<Settings>) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [restarted, setRestarted] = useState(false);
  if (!settings) return null;
  const system =
    settings.goggles === "auto" &&
    status?.goggles_mode &&
    status.goggles_mode !== "auto"
      ? status.goggles_mode
      : null;

  async function choose(value: GogglesMode) {
    setBusy(true);
    setError("");
    setRestarted(false);
    try {
      await onSave({ goggles: value });
      setRestarted(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel goggles">
      <PanelHead icon={<IconGoggles />} title="Goggles" wiki="goggles" />
      <label htmlFor="goggles-type">Goggles type</label>
      <select
        id="goggles-type"
        disabled={disabled || busy}
        value={settings.goggles}
        onChange={(e) => void choose(e.target.value as GogglesMode)}
      >
        {CHOICES.map((c) => (
          <option key={c.value} value={c.value}>
            {c.label}
          </option>
        ))}
      </select>
      {detected && (
        <p className="help">
          Right now: <strong>{detected}</strong>
        </p>
      )}
      {system && (
        <p className="help">
          System default: {SYSTEM_NAMES[system] ?? system}.
        </p>
      )}
      {restarted && <p className="help">Saved. Receiver restarting…</p>}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <p className="help">
        Automatic works for most goggles. Not found? Pick the type.
      </p>
    </section>
  );
}
