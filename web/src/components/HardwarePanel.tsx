import { Fragment, useEffect, useState } from "react";
import type { ButtonAction, GpioButton, Settings } from "../types";
import { api, headers } from "../api";
import { PanelHead } from "./PanelHead";
import { IconChip, IconCheck, IconBell, IconTrash } from "./icons";

type ButtonDraft = {
  pin: string;
  action: ButtonAction;
  hold: string;
  script: string;
};
type Draft = {
  buttons: ButtonDraft[];
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
const MAX_BUTTONS = 4;
const ACTIONS: { value: ButtonAction; label: string; hold: string }[] = [
  { value: "shutdown", label: "Shut down the Pi", hold: "2000" },
  { value: "reboot", label: "Reboot the Pi", hold: "2000" },
  { value: "save_clip", label: "Save the last 30 seconds", hold: "0" },
  { value: "toggle_recording", label: "Start / stop recording", hold: "0" },
  { value: "restart_receiver", label: "Restart the receiver", hold: "1000" },
  { value: "script", label: "Run a script", hold: "0" },
];
const HOLDS = [
  { value: "0", label: "Tap" },
  { value: "1000", label: "Hold 1 s" },
  { value: "2000", label: "Hold 2 s" },
  { value: "3000", label: "Hold 3 s" },
  { value: "5000", label: "Hold 5 s" },
];
const SCRIPT_NAME = /^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$/;
function buttonDraftOf(b: GpioButton): ButtonDraft {
  return {
    pin: String(b.pin),
    action: b.action,
    hold: String(b.hold_ms),
    script: b.script ?? "",
  };
}
function draftOf(s: Settings): Draft {
  return {
    buttons: (s.buttons ?? []).map(buttonDraftOf),
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
  onSave: (patch: Partial<Settings>) => Promise<void>;
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
    for (const [i, b] of draft.buttons.entries()) {
      if (pin(b.pin) === null) {
        setError(
          `Button ${i + 1}: the pin must be a GPIO number between 2 and 27.`,
        );
        return;
      }
      if (b.action === "script" && !SCRIPT_NAME.test(b.script)) {
        setError(
          `Button ${i + 1}: the script name can only have letters, digits, - _ and .`,
        );
        return;
      }
    }
    const used = [
      ...FIELDS.filter((f) => draft[f.enabled]).map((f) => pin(draft[f.pin])),
      ...draft.buttons.map((b) => pin(b.pin)),
    ];
    if (new Set(used).size !== used.length) {
      setError("GPIO pins must all be different.");
      return;
    }
    setSaving(true);
    try {
      await onSave({
        beeper_pin: draft.beeperEnabled ? pin(draft.beeperPin) : null,
        power_button_pin: draft.buttonEnabled ? pin(draft.buttonPin) : null,
        goggles_led_pin: draft.gogglesLedEnabled
          ? pin(draft.gogglesLedPin)
          : null,
        video_led_pin: draft.videoLedEnabled ? pin(draft.videoLedPin) : null,
        hdmi_led_pin: draft.hdmiLedEnabled ? pin(draft.hdmiLedPin) : null,
        beep_until_time_synced: draft.beepUntilTimeSynced,
        buttons: draft.buttons.map((b) => ({
          pin: pin(b.pin) as number,
          action: b.action,
          hold_ms: Number(b.hold),
          script: b.action === "script" ? b.script : null,
        })),
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
        title="Beeper, LEDs & buttons"
        wiki="hardware"
      />
      <p className="help">
        BCM pin numbers.{" "}
        <a href="/wiki.html#gpio-pinout" target="_blank" rel="noreferrer">
          Pinout &amp; wiring
        </a>
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
      <h3 className="subhead">Extra buttons</h3>
      <p className="help">
        Switch between a GPIO pin and GND, one action each.{" "}
        <a href="/wiki.html#extra-buttons" target="_blank" rel="noreferrer">
          Wiring
        </a>
      </p>
      {draft.buttons.length > 0 && (
        <div className="button-rows">
          {draft.buttons.map((b, i) => {
            const set = (patch: Partial<ButtonDraft>) =>
              setDraft({
                ...draft,
                buttons: draft.buttons.map((x, j) =>
                  j === i ? { ...x, ...patch } : x,
                ),
              });
            return (
              <div className="button-row" key={i}>
                <input
                  type="number"
                  min={2}
                  max={27}
                  placeholder="GPIO"
                  aria-label={`Button ${i + 1} GPIO pin`}
                  disabled={disabled}
                  value={b.pin}
                  onChange={(e) => set({ pin: e.target.value })}
                />
                <select
                  aria-label={`Button ${i + 1} action`}
                  disabled={disabled}
                  value={b.action}
                  onChange={(e) => {
                    const action = e.target.value as ButtonAction;
                    set({
                      action,
                      hold: ACTIONS.find((a) => a.value === action)!.hold,
                    });
                  }}
                >
                  {ACTIONS.map((a) => (
                    <option key={a.value} value={a.value}>
                      {a.label}
                    </option>
                  ))}
                </select>
                <select
                  aria-label={`Button ${i + 1} hold time`}
                  disabled={disabled}
                  value={b.hold}
                  onChange={(e) => set({ hold: e.target.value })}
                >
                  {HOLDS.map((h) => (
                    <option key={h.value} value={h.value}>
                      {h.label}
                    </option>
                  ))}
                </select>
                {b.action === "script" && (
                  <input
                    type="text"
                    maxLength={64}
                    placeholder="script file name"
                    aria-label={`Button ${i + 1} script file name`}
                    disabled={disabled}
                    value={b.script}
                    onChange={(e) => set({ script: e.target.value })}
                  />
                )}
                <button
                  type="button"
                  aria-label={`Remove button ${i + 1}`}
                  disabled={disabled}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      buttons: draft.buttons.filter((_, j) => j !== i),
                    })
                  }
                >
                  <IconTrash />
                </button>
              </div>
            );
          })}
        </div>
      )}
      <div className="actions">
        <button
          type="button"
          disabled={disabled || draft.buttons.length >= MAX_BUTTONS}
          onClick={() =>
            setDraft({
              ...draft,
              buttons: [
                ...draft.buttons,
                { pin: "", action: "save_clip", hold: "0", script: "" },
              ],
            })
          }
        >
          Add a button
        </button>
        <button
          type="button"
          title="Shutdown on a 2 s hold; the same switch starts the Pi again"
          disabled={
            disabled ||
            draft.buttons.length >= MAX_BUTTONS ||
            draft.buttons.some((b) => b.pin === "3")
          }
          onClick={() =>
            setDraft({
              ...draft,
              buttons: [
                ...draft.buttons,
                { pin: "3", action: "shutdown", hold: "2000", script: "" },
              ],
            })
          }
        >
          Add power button (GPIO 3)
        </button>
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
        Beeps and LEDs follow the goggles, video and HDMI state.
      </p>
    </section>
  );
}
