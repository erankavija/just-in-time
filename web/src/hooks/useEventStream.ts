import { useEffect, useRef, useState } from 'react';
import { API_BASE } from '../api/client';

interface UseEventStreamOptions {
  onChanged: (version: number) => void;
  /** Minimum milliseconds between UI refreshes. Defaults to 1000. */
  minIntervalMs?: number;
}

/**
 * Connects to the server's SSE endpoint and calls `onChanged` when the
 * `.jit/` directory changes. EventSource auto-reconnects on failure.
 *
 * Events are throttled so that `onChanged` fires at most once per
 * `minIntervalMs` (default 1 s) regardless of how many SSE messages arrive.
 * The latest version seen during a throttle window is always delivered.
 */
export function useEventStream({ onChanged, minIntervalMs = 1000 }: UseEventStreamOptions) {
  const [connected, setConnected] = useState(false);
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  }, [onChanged]);

  // Throttle state kept in refs so the effect closure stays stable.
  const pendingVersionRef = useRef<number | null>(null);
  const lastFiredRef = useRef<number>(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const fire = (version: number) => {
      lastFiredRef.current = Date.now();
      pendingVersionRef.current = null;
      onChangedRef.current(version);
    };

    const handleChange = (version: number) => {
      // Always track the latest version seen.
      pendingVersionRef.current = version;

      const elapsed = Date.now() - lastFiredRef.current;
      if (elapsed >= minIntervalMs) {
        // Enough time has passed — fire immediately.
        if (timerRef.current !== null) {
          clearTimeout(timerRef.current);
          timerRef.current = null;
        }
        fire(version);
      } else {
        // Still within the throttle window — schedule delivery for when it expires.
        if (timerRef.current === null) {
          timerRef.current = setTimeout(() => {
            timerRef.current = null;
            const v = pendingVersionRef.current;
            if (v !== null) fire(v);
          }, minIntervalMs - elapsed);
        }
      }
    };

    const source = new EventSource(`${API_BASE}/events/stream`);

    source.addEventListener('open', () => {
      setConnected(true);
    });

    source.addEventListener('change', (e: MessageEvent) => {
      try {
        const data = JSON.parse(e.data);
        if (typeof data.version === 'number') {
          handleChange(data.version);
        }
      } catch {
        // Ignore malformed events
      }
    });

    source.addEventListener('error', () => {
      setConnected(false);
    });

    return () => {
      source.close();
      setConnected(false);
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [minIntervalMs]);

  return { connected };
}
