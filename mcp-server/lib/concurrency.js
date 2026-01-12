/**
 * Concurrency limiter
 * 
 * Limits the number of concurrent CLI command executions to prevent
 * uncontrolled parallel spawning.
 */

/**
 * Simple concurrency limiter using a queue
 */
export class ConcurrencyLimiter {
  /**
   * @param {number} maxConcurrent - Maximum concurrent operations
   */
  constructor(maxConcurrent = 10) {
    this.maxConcurrent = maxConcurrent;
    this.running = 0;
    this.queue = [];
  }
  
  /**
   * Execute a function with concurrency limiting
   * @template T
   * @param {() => Promise<T>} fn - Async function to execute
   * @returns {Promise<T>}
   */
  async run(fn) {
    // Wait if at capacity
    if (this.running >= this.maxConcurrent) {
      await new Promise(resolve => this.queue.push(resolve));
    }
    
    this.running++;
    
    try {
      return await fn();
    } finally {
      this.running--;
      
      // Start next queued operation
      if (this.queue.length > 0) {
        const resolve = this.queue.shift();
        // Resolve in next tick to avoid stack overflow
        setImmediate(resolve);
      }
    }
  }
  
  /**
   * Get current stats
   * @returns {{running: number, queued: number}}
   */
  getStats() {
    return {
      running: this.running,
      queued: this.queue.length,
    };
  }
}
