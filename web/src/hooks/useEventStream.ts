import { useEffect, useRef, useState } from 'react';
import { API_BASE } from '../api/client';

interface UseEventStreamOptions {
  onChanged: (version: number) => void;
}

/**
 * Connects to the server's SSE endpoint and calls `onChanged` when the
 * `.jit/` directory changes. EventSource auto-reconnects on failure.
 */
export function useEventStream({ onChanged }: UseEventStreamOptions) {
  const [connected, setConnected] = useState(false);
  const onChangedRef = useRef(onChanged);
  useEffect(() => {
    onChangedRef.current = onChanged;
  }, [onChanged]);

  useEffect(() => {
    const source = new EventSource(`${API_BASE}/events/stream`);

    source.addEventListener('open', () => {
      setConnected(true);
    });

    source.addEventListener('change', (e: MessageEvent) => {
      try {
        const data = JSON.parse(e.data);
        if (typeof data.version === 'number') {
          onChangedRef.current(data.version);
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
    };
  }, []);

  return { connected };
}
