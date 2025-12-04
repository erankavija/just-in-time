import { execSync } from 'child_process';

// Test what command gets generated
const args = {
  query: "priority"
};

const cliArgs = `search "${args.query}" --json`;
const cmd = `../target/debug/jit ${cliArgs}`;

console.log("Command:", cmd);

try {
  const result = execSync(cmd, { encoding: 'utf-8', cwd: '/home/vkaskivuo/Projects/just-in-time' });
  console.log("Result:", result);
} catch (err) {
  console.log("Error:", err.message);
  console.log("Stdout:", err.stdout);
  console.log("Stderr:", err.stderr);
}
