import { createContext } from "react";

/** Re-reads the briefings and Needs-you list (after something was changed in the vault). Provided by App. */
export const RefreshContext = createContext<() => void>(() => {});
