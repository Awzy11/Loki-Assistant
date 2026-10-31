import { useEffect, useRef, useState } from "react";
import { transcribeAudio } from "./transport";

const MAX_SECONDS = 300;

/** Push to record, push to stop: the recording goes to the speech-to-text model on the model server and the
 * transcript comes back as text for the input box (it is not sent automatically). */
export default function Mic({ onText, onError, disabled }: { onText: (text: string) => void; onError: (message: string) => void; disabled?: boolean }) {
  const [state, setState] = useState<"idle" | "recording" | "working">("idle");
  const [secs, setSecs] = useState(0);
  const recorder = useRef<MediaRecorder | null>(null);
  const stream = useRef<MediaStream | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(
    () => () => {
      window.clearInterval(timer.current);
      stream.current?.getTracks().forEach((t) => t.stop());
    },
    [],
  );

  if (typeof navigator === "undefined" || !navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") return null;

  async function start() {
    try {
      const media = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
      stream.current = media;
      const mime = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg;codecs=opus"].find((m) => MediaRecorder.isTypeSupported(m));
      const rec = new MediaRecorder(media, mime ? { mimeType: mime } : undefined);
      const chunks: Blob[] = [];
      rec.ondataavailable = (e) => e.data.size > 0 && chunks.push(e.data);
      rec.onstop = async () => {
        window.clearInterval(timer.current);
        media.getTracks().forEach((t) => t.stop());
        const blob = new Blob(chunks, { type: rec.mimeType || mime || "audio/webm" });
        if (blob.size < 1500) {
          setState("idle");
          onError("That recording was too short to transcribe.");
          return;
        }
        setState("working");
        try {
          const text = await transcribeAudio(blob);
          if (text) onText(text);
          else onError("I couldn't hear any speech in that recording.");
        } catch (e) {
          onError(String(e instanceof Error ? e.message : e));
        } finally {
          setState("idle");
        }
      };
      recorder.current = rec;
      rec.start();
      setSecs(0);
      setState("recording");
      timer.current = window.setInterval(() => {
        setSecs((s) => {
          if (s + 1 >= MAX_SECONDS) rec.state === "recording" && rec.stop();
          return s + 1;
        });
      }, 1000);
    } catch (e) {
      onError(e instanceof DOMException && e.name === "NotAllowedError" ? "Microphone access was blocked — allow it for this site in your browser's settings." : `Couldn't start recording: ${e instanceof Error ? e.message : e}`);
    }
  }

  function stop() {
    if (recorder.current?.state === "recording") recorder.current.stop();
  }

  const mmss = `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
  return (
    <button
      type="button"
      className={`loki-mic ${state === "recording" ? "loki-mic-on" : ""}`}
      onClick={state === "recording" ? stop : start}
      disabled={disabled || state === "working"}
      title={state === "recording" ? "Stop recording and transcribe" : "Record a voice message (transcribed on your speech server)"}
      aria-label={state === "recording" ? "Stop recording" : "Record a voice message"}
    >
      {state === "working" ? "…" : state === "recording" ? `■ ${mmss}` : "🎤"}
    </button>
  );
}
