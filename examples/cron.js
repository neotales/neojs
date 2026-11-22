import { cron, next } from "neo:cron";

// `next` is a pure UTC query: it creates no job and keeps nothing alive.
const upcoming = next("*/5 * * * *");
console.log(`next five-minute boundary (UTC): ${upcoming.toISOString()}`);

// A job keeps the process alive until it is disposed. This example schedules
// one and immediately disposes it, so it exits; remove the dispose to let the
// callback run every five minutes.
const job = cron("*/5 * * * *", () => {
    console.log("tick");
});
job.dispose();
