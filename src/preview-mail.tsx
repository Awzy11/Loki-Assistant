// Dev-only: the mail dashboard with invented sample messages (no real mail). npm run dev → /preview-mail.html
import { createRoot } from "react-dom/client";
import "./App.css";
import MailDashboard from "./MailDashboard";

const now = Date.now();
const d = (days: number) => now - days * 86_400_000;
const mk = (account: string, id: string, from: string, subject: string, days: number, unread: boolean, category: string, urgency: string, cls: string, why: string, extra: object = {}) =>
  ({ account, id, from, subject, at_ms: d(days), unread, snippet: "", category, urgency, class: cls, reply: false, why, quadrant: "", gain: "", next: "", ...extra });
const A = "sample.one@example.com", B = "sample.two@example.com";
const items = [
  mk(A, "1", "Example Bank <alerts@bank.example>", "Statement ready for October", 0.2, true, "finance", "routine", "receipt", "monthly statement", { quadrant: "q4" }),
  mk(A, "2", "Course Office <office@school.example>", "Assignment 2304 due Friday", 1, false, "school", "urgent", "action", "deadline this week", { quadrant: "q1", gain: "a", next: "open the prompt, outline in 30 minutes" }),
  mk(A, "3", "Property Manager <pm@housing.example>", "Please confirm inspection time", 2, false, "home", "priority", "action", "needs a reply", { reply: true, quadrant: "q2", gain: "i", next: "pick a slot and reply" }),
  mk(A, "4", "Broker <noreply@broker.example>", "Your quarterly summary", 5, true, "investments", "routine", "fyi", "portfolio summary", { quadrant: "q4" }),
  mk(A, "5", "Security <no-reply@service.example>", "New sign-in from a new device", 0.5, true, "cyber", "urgent", "action", "check the sign-in"),
  mk(A, "6", "Shop <deals@shop.example>", "Weekend sale: 40% off", 3, true, "other", "discard", "junk", "promotion"),
  mk(B, "7", "Unit Admin <admin@unit.example>", "Update to the Monday schedule", 1, true, "work", "priority", "review", "schedule change", { quadrant: "q2", gain: "n", next: "read and adjust the week" }),
  mk(B, "8", "Dev Platform <team@platform.example>", "Your API usage this month", 6, false, "tech", "routine", "fyi", "usage report", { quadrant: "q4" }),
  mk(B, "9", "Recruiter <hello@jobs.example>", "Following up on your profile", 12, false, "work", "priority", "action", "waiting on a reply", { reply: true, quadrant: "q3", next: "decline politely" }),
  mk(B, "10", "Family <family@mail.example>", "Photos from the weekend", 4, true, "home", "routine", "fyi", "family photos"),
  mk(A, "12", "Shop <orders@shop.example>", "Your order has shipped", 20, false, "finance", "routine", "receipt", "shipping notice", { archived: true }),
  mk(B, "13", "Unit Admin <admin@unit.example>", "Fitness report timeline (archived)", 40, false, "work", "priority", "action", "still needs a reply", { archived: true, reply: true }),
  mk(B, "11", "Insurance <service@insure.example>", "Policy renewal notice", 9, false, "finance", "priority", "action", "renewal due soon", { quadrant: "q2", gain: "g", next: "compare quotes and renew" }),
];
const page = { status: "done", phase: "Finished", done: 0, total: 0, error: "", updated: Math.floor(now / 1000) - 300, items };
const real = window.fetch.bind(window);
window.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
  const u = String(input);
  if (u.includes("/api/v1/mail/inbox/message")) return Promise.resolve(new Response(JSON.stringify({ account: A, id: "2", from: "Course Office <office@school.example>", to: A, cc: "", subject: "Assignment 2304 due Friday", date: "Mon, 26 Oct 2026 09:12:00 -0400", body: "Hello,\n\nThis is a reminder that the 2304 writing assignment is due Friday at 1700.\nSubmission link: https://school.example/submit/2304\n\nThanks,\nCourse Office", attachments: ["Prompt.pdf"], unread: false, link: "https://mail.google.com/" })));
  if (u.includes("/api/v1/mail/inbox/draft")) return Promise.resolve(new Response(JSON.stringify({ draft: "Hi,\n\nThursday at 10:00 works for me.\n\nSam", saved: "Saved as a draft in sample.one@example.com. Nothing was sent." })));
  if (u.includes("/api/v1/mail/inbox")) return Promise.resolve(new Response(JSON.stringify(page)));
  return real(input, init);
};
try { localStorage.setItem("loki_api_token", "preview"); } catch { /* ignore */ }
createRoot(document.getElementById("root")!).render(<div className="loki-shell"><MailDashboard /></div>);
if (location.search.includes("open=1")) setTimeout(() => (document.querySelectorAll(".mx-open")[1] as HTMLElement | undefined)?.click(), 600);
if (location.search.includes("archived=1")) setTimeout(() => (document.querySelectorAll(".mx-scope button")[1] as HTMLElement | undefined)?.click(), 700);
