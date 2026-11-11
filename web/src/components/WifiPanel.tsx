import { useEffect, useState } from "react";
import type { Settings, Wifi, WifiClient, WifiNetwork } from "../types";
import { api, headers, usePolling, withTimeout } from "../api";
import { PanelHead } from "./PanelHead";
import { IconWifi, IconCheck, IconClose } from "./icons";
export function WifiPanel({
  notify,
  disabled = false,
  settings,
  onSave,
}: {
  notify: (s: string) => void;
  disabled?: boolean;
  settings: Settings | null;
  onSave: (patch: Partial<Settings>) => Promise<void>;
}) {
  const { value: wifi } = usePolling<Wifi>("wifi", 5000);
  const { value: client } = usePolling<WifiClient>("wifi/client", 3000);
  const [ssid, setSsid] = useState("");
  const [password, setPassword] = useState("");
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [joinSsid, setJoinSsid] = useState("");
  const [joinPassword, setJoinPassword] = useState("");
  const [joinBusy, setJoinBusy] = useState(false);
  const [leaveBusy, setLeaveBusy] = useState(false);
  const [alwaysApBusy, setAlwaysApBusy] = useState(false);
  const [networks, setNetworks] = useState<WifiNetwork[] | null>(null);
  const [scanBusy, setScanBusy] = useState(false);
  const [error, setError] = useState("");
  async function scan() {
    setScanBusy(true);
    setError("");
    const request = withTimeout(null, 15000);
    try {
      const r = await api<{ networks: WifiNetwork[] }>("wifi/scan", {
        signal: request.signal,
      });
      setNetworks(r.networks);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Scan failed.");
    } finally {
      request.done();
      setScanBusy(false);
    }
  }
  useEffect(() => {
    if (wifi?.ssid && !dirty) setSsid(wifi.ssid);
  }, [wifi?.ssid, dirty]);
  async function setAlwaysAp(wifi_always_ap: boolean) {
    if (!settings) return;
    setAlwaysApBusy(true);
    setError("");
    try {
      await onSave({ wifi_always_ap });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setAlwaysApBusy(false);
    }
  }
  return (
    <section className="panel wifi">
      <PanelHead icon={<IconWifi />} title="Wi-Fi access point" wiki="wifi" />
      {wifi?.available ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            setBusy(true);
            setError("");
            void api<{ message: string }>("wifi", {
              method: "POST",
              headers,
              body: JSON.stringify({ ssid, password }),
            })
              .then((r) => {
                notify(r.message);
                setPassword("");
                setDirty(false);
              })
              .catch((e) => setError(String(e)))
              .finally(() => setBusy(false));
          }}
        >
          <div className="wifi-fields">
            <label>
              SSID
              <input
                value={ssid}
                required
                maxLength={32}
                autoComplete="off"
                onChange={(e) => {
                  setSsid(e.target.value);
                  setDirty(true);
                }}
              />
            </label>
            <label>
              New password
              <input
                type="password"
                value={password}
                required
                minLength={8}
                maxLength={63}
                autoComplete="new-password"
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <button disabled={busy || disabled || wifi.applying}>
              <IconCheck /> Apply Wi-Fi
            </button>
          </div>
          <p className="help">
            Applying disconnects Wi-Fi. Reconnect using the new SSID and
            password. HDMI keeps running.
          </p>
        </form>
      ) : (
        <p className="help">No managed access point configured.</p>
      )}
      {wifi?.available && (
        <>
          <hr />
          <p>
            <strong>Join an existing Wi-Fi network</strong>
          </p>
          {client?.active ? (
            <div>
              <p>
                Connected to <strong>{client.ssid ?? "…"}</strong>
                {client.ip ? ` — IP ${client.ip}` : " — connecting…"}
              </p>
              <button
                disabled={disabled || leaveBusy || client.applying}
                onClick={() => {
                  setError("");
                  setLeaveBusy(true);
                  void api<{ message: string }>("wifi/client", {
                    method: "DELETE",
                    headers,
                  })
                    .then((r) => notify(r.message))
                    .catch((e) => setError(String(e)))
                    .finally(() => setLeaveBusy(false));
                }}
              >
                <IconClose /> Leave, back to access point
              </button>
            </div>
          ) : (
            <form
              onSubmit={(e) => {
                e.preventDefault();
                setJoinBusy(true);
                setError("");
                void api<{ message: string }>("wifi/client", {
                  method: "POST",
                  headers,
                  body: JSON.stringify({
                    ssid: joinSsid,
                    password: joinPassword,
                  }),
                })
                  .then((r) => {
                    notify(r.message);
                    setJoinPassword("");
                  })
                  .catch((e) => setError(String(e)))
                  .finally(() => setJoinBusy(false));
              }}
            >
              <div className="wifi-fields">
                <label>
                  Nearby networks
                  <select
                    value=""
                    disabled={scanBusy || disabled}
                    onChange={(e) => {
                      if (e.target.value) setJoinSsid(e.target.value);
                    }}
                  >
                    <option value="">
                      {scanBusy
                        ? "Scanning…"
                        : networks
                          ? networks.length > 0
                            ? "Pick a network…"
                            : "No networks found"
                          : "Scan first, or type the SSID below"}
                    </option>
                    {networks?.map((n) => (
                      <option key={n.ssid} value={n.ssid}>
                        {n.secured ? "🔒 " : ""}
                        {n.ssid} ({n.signal}%)
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  type="button"
                  disabled={scanBusy || disabled}
                  onClick={() => void scan()}
                >
                  <IconWifi /> {scanBusy ? "Scanning…" : "Scan for networks"}
                </button>
                <label>
                  Network SSID
                  <input
                    value={joinSsid}
                    required
                    maxLength={32}
                    autoComplete="off"
                    onChange={(e) => setJoinSsid(e.target.value)}
                  />
                </label>
                <label>
                  Password
                  <input
                    type="password"
                    value={joinPassword}
                    maxLength={63}
                    autoComplete="off"
                    placeholder="Leave empty for an open network"
                    onChange={(e) => setJoinPassword(e.target.value)}
                  />
                </label>
                <button disabled={joinBusy || disabled || client?.applying}>
                  <IconCheck /> Join network
                </button>
              </div>
              <p className="help">
                Turns the access point off. Find the Pi at{" "}
                <code>ungoggled.local</code> or on the HDMI screen. Falls back
                after 30 s.
              </p>
            </form>
          )}
          <ul className="toggle-list">
            <li>
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settings?.wifi_always_ap ?? true}
                  disabled={disabled || alwaysApBusy || !settings}
                  onChange={(e) => void setAlwaysAp(e.currentTarget.checked)}
                />
                Always broadcast the access point after a reboot
              </label>
            </li>
          </ul>
        </>
      )}
      {(error || wifi?.error || client?.error) && (
        <p role="alert" className="error">
          {error || wifi?.error || client?.error}
        </p>
      )}
    </section>
  );
}
