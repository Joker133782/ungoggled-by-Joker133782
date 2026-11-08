import { Fragment, useEffect, useState } from "react";
import type { Settings } from "../types";
import { api, headers } from "../api";
import { PanelHead } from "./PanelHead";
import { IconChip, IconCheck, IconBell } from "./icons";

type Draft = {
  beeperEnabled: boolean;
  beeperPin: string;
  buttonEnabled: boolean;
  buttonPin: string;
  gogglesLedEnabled: boolean;
  gogglesLedPin: string;
  videoLedEnabled: boolean;
  videoLedPin: string;
  hdmiLedEnabled: boolean;
  hdmiLedPin: string;
  beepUntilTimeSynced: boolean;
};
function draftOf(s: Settings): Draft {
  return {
    beeperEnabled: s.beeper_pin != null,
    beeperPin: s.beeper_pin != null ? String(s.beeper_pin) : "",
    buttonEnabled: s.power_button_pin != null,
    buttonPin: s.power_button_pin != null ? String(s.power_button_pin) : "",
    gogglesLedEnabled: s.goggles_led_pin != null,
    gogglesLedPin: s.goggles_led_pin != null ? String(s.goggles_led_pin) : "",
    videoLedEnabled: s.video_led_pin != null,
    videoLedPin: s.video_led_pin != null ? String(s.video_led_pin) : "",
    hdmiLedEnabled: s.hdmi_led_pin != null,
    hdmiLedPin: s.hdmi_led_pin != null ? String(s.hdmi_led_pin) : "",
    beepUntilTimeSynced: s.beep_until_time_synced,
  };
}
function pin(value: string): number | null {
  const n = Number(value);
  return value !== "" && Number.isInteger(n) && n >= 2 && n <= 27 ? n : null;
}
const FIELDS = [
  {
    enabled: "beeperEnabled",
    pin: "beeperPin",
    defaultPin: 6,
    label: "Piezo beeper on GPIO",
    error: "Beeper pin must be a GPIO number between 2 and 27.",
  },
  {
    enabled: "buttonEnabled",
    pin: "buttonPin",
    defaultPin: 19,
    label: "Shutdown button on GPIO",
    error: "Power button pin must be a GPIO number between 2 and 27.",
  },
  {
    enabled: "gogglesLedEnabled",
    pin: "gogglesLedPin",
    defaultPin: 17,
    label: "Goggles LED on GPIO",
    error: "Goggles LED pin must be a GPIO number between 2 and 27.",
  },
  {
    enabled: "videoLedEnabled",
    pin: "videoLedPin",
    defaultPin: 23,
    label: "Video LED on GPIO",
    error: "Video LED pin must be a GPIO number between 2 and 27.",
  },
  {
    enabled: "hdmiLedEnabled",
    pin: "hdmiLedPin",
    defaultPin: 24,
    label: "HDMI LED on GPIO",
    error: "HDMI LED pin must be a GPIO number between 2 and 27.",
  },
] as const;
export function HardwarePanel({
  settings,
  onSave,
  disabled,
}: {
  settings: Settings | null;
  onSave: (next: Settings) => Promise<void>;
  disabled: boolean;
}) {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (settings && !draft) setDraft(draftOf(settings));
  }, [settings, draft]);
  if (!settings || !draft) return null;
  const dirty = JSON.stringify(draft) !== JSON.stringify(draftOf(settings));
  async function apply() {
    if (!settings || !draft) return;
    setError("");
    for (const f of FIELDS) {
      if (draft[f.enabled] && pin(draft[f.pin]) === null) {
        setError(f.error);
        return;
      }
    }
    const used = FIELDS.filter((f) => draft[f.enabled]).map((f) =>
      pin(draft[f.pin]),
    );
    if (new Set(used).size !== used.length) {
      setError("GPIO pins must all be different.");
      return;
    }
    setSaving(true);
    try {
      await onSave({
        ...settings,
        beeper_pin: draft.beeperEnabled ? pin(draft.beeperPin) : null,
        power_button_pin: draft.buttonEnabled ? pin(draft.buttonPin) : null,
        goggles_led_pin: draft.gogglesLedEnabled
          ? pin(draft.gogglesLedPin)
          : null,
        video_led_pin: draft.videoLedEnabled ? pin(draft.videoLedPin) : null,
        hdmi_led_pin: draft.hdmiLedEnabled ? pin(draft.hdmiLedPin) : null,
        beep_until_time_synced: draft.beepUntilTimeSynced,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }
  async function testBeeper() {
    setError("");
    setTesting(true);
    try {
      await api("hardware/test-beeper", { method: "POST", headers });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTesting(false);
    }
  }
  return (
    <section className="panel hardware">
      <PanelHead
        icon={<IconChip />}
        title="Beeper, LEDs & power button"
        wiki="hardware"
      />
      <p className="help">
        GPIO pins use BCM numbering. Defaults are piezo GPIO6, shutdown button
        GPIO19 (NuclearHazard V5 hat), goggles LED GPIO17, video LED GPIO23 and
        HDMI LED GPIO24. The three LEDs are plain (non-addressable) LEDs, each
        wired to its own GPIO with a series resistor; each simply lights up
        while its state is true.
      </p>
      <div className="hardware-fields">
        {FIELDS.map((f) => (
          <Fragment key={f.enabled}>
            <label className="toggle">
              <input
                type="checkbox"
                checked={draft[f.enabled]}
                disabled={disabled}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    [f.enabled]: e.target.checked,
                    ...(e.target.checked && draft[f.pin] === ""
                      ? { [f.pin]: String(f.defaultPin) }
                      : {}),
                  })
                }
              />
              {f.label}
            </label>
            <input
              type="number"
              min={2}
              max={27}
              aria-label={f.label}
              disabled={disabled || !draft[f.enabled]}
              value={draft[f.pin]}
              onChange={(e) => setDraft({ ...draft, [f.pin]: e.target.value })}
            />
          </Fragment>
        ))}
      </div>
      <label className="toggle">
        <input
          type="checkbox"
          checked={draft.beepUntilTimeSynced}
          disabled={disabled}
          onChange={(e) =>
            setDraft({ ...draft, beepUntilTimeSynced: e.target.checked })
          }
        />
        Beep every 5 seconds until the clock is synced
      </label>
      <div className="actions">
        <button
          disabled={disabled || saving || !dirty}
          onClick={() => void apply()}
        >
          <IconCheck /> Apply
        </button>
        <button
          disabled={disabled || testing || dirty || settings.beeper_pin == null}
          title={
            settings.beeper_pin == null
              ? "Enable the beeper and apply a GPIO pin first"
              : dirty
                ? "Apply your changes first"
                : undefined
          }
          onClick={() => void testBeeper()}
        >
          <IconBell /> {testing ? "Beeping…" : "Test buzzer"}
        </button>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <p className="help">
        Goggles, video and HDMI each beep when they connect or disconnect; the
        receiver also beeps once at startup. Happy and sad tones are
        distinguishable, as are the three events. Each LED lights up while its
        own state is true and turns off otherwise. The clock reminder beeps
        until a browser has set the Pi clock (or NTP has synced it) since boot,
        so recordings get correct timestamps.
      </p>
    </section>
  );
}
