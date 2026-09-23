import { useRef } from "react";

/**
 * Keeps the last non-null/undefined value
 * so a Dialog keeps showing its content during the close animation
 *
 * @param value current value
 * @returns the current value (if valid) or the last valid value
 */
export function useLastValidValue<T>(value: T | null | undefined): T | null {
  const ref = useRef<T | null>(null);

  // Update the ref synchronously (during render, not in useEffect)
  if (value != null) {
    ref.current = value;
  }

  // Return the current value or the last valid one
  return value ?? ref.current;
}
