// derive_vault.js
const { PublicKey } = require("@solana/web3.js");
const PROGRAM_ID = new PublicKey("8XdQXT8TiguCysUPoAzXV611mhdgYKAN6G331CXC81GP");
const [vault, bump] = PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID);
console.log("VAULT_PDA", vault.toBase58(), "BUMP", bump);
