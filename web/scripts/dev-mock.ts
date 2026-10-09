// `bun run dev:mock`: the mock API on :8081 plus the Vite dev server (which proxies /api to it).
const children = [
  Bun.spawn(["bun", "scripts/mock-api.ts"], { stdio: ["inherit", "inherit", "inherit"] }),
  Bun.spawn(["bun", "run", "dev"], { stdio: ["inherit", "inherit", "inherit"] }),
];
const stop = () => children.forEach((child) => child.kill());
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
await Promise.race(children.map((child) => child.exited));
stop();

export {};
