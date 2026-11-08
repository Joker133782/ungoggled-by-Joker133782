import { useEffect, useMemo, useRef, useState } from "react";
import type { WifiShare } from "../types";
import { api } from "../api";
import { qrSvgPath, wifiQrText } from "../wifi-qr";
import { IconCheck } from "./icons";

/** A QR code as crisp black-on-white SVG, with the standard quiet zone. */
function QrCode({ text, label }: { text: string; label: string }) {
  const { size, path } = useMemo(() => qrSvgPath(text), [text]);
  return (
    <svg
      className="qr"
      viewBox={`0 0 ${size} ${size}`}
      role="img"
      aria-label={label}
      shapeRendering="crispEdges"
    >
      <rect width={size} height={size} fill="#fff" />
      <path d={path} fill="#000" />
    </svg>
  );
}

/**
 * Share sheet for spectators. Getting a phone to the stream really is two
 * separate steps, and the sheet says so instead of pretending one code does
 * it: first join the Pi's Wi-Fi (a Wi-Fi QR code, with the current name and
 * password from the Pi), then open the stream page (a second QR code). Both
 * codes are drawn right here, without any outside service.
 */
export default function ShareSheet({
  streamOn,
  onClose,
}: {
  streamOn: boolean;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [share, setShare] = useState<WifiShare | null>(null);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const watchUrl = `${window.location.origin}/watch.html`;
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => dialog?.close();
  }, []);
  useEffect(() => {
    api<WifiShare>("wifi/share", { cache: "no-store" })
      .then(setShare)
      .catch((e) =>
        setError(e instanceof Error ? e.message : "Couldn't read the Wi-Fi."),
      );
  }, []);
  async function copy() {
    try {
      await navigator.clipboard.writeText(watchUrl);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      /* The address stays visible to copy by hand. */
    }
  }
  return (
    <dialog
      ref={ref}
      className="confirm share-sheet"
      aria-labelledby="share-title"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <h3 id="share-title">Let spectators watch</h3>
      <p>
        Two steps on their phone, in this order. Show them this screen, or read
        it out.
      </p>
      {!streamOn && (
        <p className="warning-text">
          The stream is off right now: switch on High or Low quality first, or
          the page just says the stream is off.
        </p>
      )}
      <ol className="share-steps">
        <li>
          <h4>
            <span className="step" aria-hidden="true">
              1
            </span>
            Join the Pi&apos;s Wi-Fi
          </h4>
          {share?.mode === "ap" && share.ssid ? (
            <>
              <QrCode
                text={wifiQrText(share.ssid, share.password)}
                label={`Wi-Fi QR code for the network ${share.ssid}`}
              />
              <dl className="share-credentials">
                <dt>Network</dt>
                <dd>{share.ssid}</dd>
                <dt>Password</dt>
                <dd>{share.password ?? "none (open network)"}</dd>
              </dl>
              <p className="share-note">
                Scan with the camera and tap Join. If the phone says this Wi-Fi
                has no internet, that&apos;s expected: stay connected.
              </p>
            </>
          ) : share?.mode === "client" ? (
            <p className="share-note">
              The Pi is on the network <strong>{share.ssid ?? "…"}</strong>{" "}
              right now, so its own Wi-Fi is off. Spectators join that network
              the usual way; there&apos;s no code for it here.
            </p>
          ) : share ? (
            <p className="share-note">
              This Pi has no Wi-Fi of its own set up. Spectators need to be on
              the same network as the Pi.
            </p>
          ) : error ? (
            <p role="alert" className="error">
              {error}
            </p>
          ) : (
            <p className="share-note">Loading…</p>
          )}
        </li>
        <li>
          <h4>
            <span className="step" aria-hidden="true">
              2
            </span>
            Then open the stream
          </h4>
          <QrCode text={watchUrl} label="QR code for the stream page" />
          <p className="share-url">
            {watchUrl.replace(/^https?:\/\//, "")}
            {navigator.clipboard && (
              <button
                type="button"
                className="link-button"
                onClick={() => void copy()}
              >
                {copied ? "Copied" : "Copy link"}
              </button>
            )}
          </p>
          <p className="share-note">
            Only works once step 1 is done: the page is on the Pi, not the
            internet.
            {window.location.protocol === "https:" &&
              " The first time, the browser warns that the connection isn't private (the Pi signs its own certificate): tap Advanced, then Proceed."}
          </p>
        </li>
      </ol>
      <div className="actions">
        <button className="primary" onClick={onClose}>
          <IconCheck /> Done
        </button>
      </div>
    </dialog>
  );
}
