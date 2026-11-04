/**
 * Waiter queue behind the one-time "custom window sizes" warning dialog.
 *
 * A launch of a fingerprinted (wayfern) profile parks on that dialog until
 * the user answers it. Two launches can park at once — a bulk run, or a
 * second Launch click before the first dialog is answered — and the dialog
 * used to keep a single shared resolver ref: the second park overwrote it,
 * the first promise lost its resolver forever and that profile stayed pinned
 * in launching until the app restarted.
 *
 * The question the dialog asks is global and one-time, so one answer applies
 * to every current waiter: Continue releases all parked launches, Cancel
 * cancels them all. A later park (after the answer, e.g. when the warning was
 * not dismissed for good) waits for the next answer instead of leaking into
 * the previous batch.
 */

export type WindowResizeWarningWaiter = (proceed: boolean) => void;

export interface WindowResizeWarningQueue {
  /** How many launches are currently parked on the dialog. */
  readonly pending: number;
  /**
   * Park the calling launch on the dialog. The returned promise settles with
   * the user's answer the next time `answer` runs — never before, and only
   * once.
   */
  park(): Promise<boolean>;
  /**
   * Settle every parked waiter with the same answer and clear the queue.
   * Returns how many waiters were released; answering with an empty queue is
   * a no-op.
   */
  answer(proceed: boolean): number;
}

export function createWindowResizeWarningQueue(): WindowResizeWarningQueue {
  let waiters: WindowResizeWarningWaiter[] = [];
  return {
    get pending() {
      return waiters.length;
    },
    park() {
      return new Promise<boolean>((resolve) => {
        waiters.push(resolve);
      });
    },
    answer(proceed: boolean) {
      const released = waiters;
      waiters = [];
      for (const resolve of released) {
        resolve(proceed);
      }
      return released.length;
    },
  };
}
