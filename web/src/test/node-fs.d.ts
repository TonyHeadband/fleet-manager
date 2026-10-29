// The one Node call the tests make (reading a stylesheet), declared here instead of depending on @types/node.
declare module "node:fs" {
  export function readFileSync(path: string, encoding: "utf8"): string;
}
