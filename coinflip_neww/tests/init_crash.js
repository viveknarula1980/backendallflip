// tests/init_crash.js
require("dotenv").config();
const anchor = require("@coral-xyz/anchor");
const { PublicKey, SystemProgram } = require("@solana/web3.js");

// ENV you must set when running:
// ANCHOR_PROVIDER_URL=...  ANCHOR_WALLET=~/.config/solana/id.json
// PROGRAM_ID=<CRASH program id>
// ADMIN_PUBKEY_BASE58=<your ed25519 backend pubkey base58>
// IDL_FILE=target/idl/crash_program.json   (default)

(async () => {
  try {
    const url = process.env.ANCHOR_PROVIDER_URL || "https://api.devnet.solana.com";
    const walletPath = process.env.ANCHOR_WALLET;
    if (!walletPath) throw new Error("Set ANCHOR_WALLET=~/.config/solana/id.json");
    if (!process.env.PROGRAM_ID) throw new Error("Set PROGRAM_ID=<Crash program id>");
    if (!process.env.ADMIN_PUBKEY_BASE58) throw new Error("Set ADMIN_PUBKEY_BASE58=<ed25519 backend pubkey base58>");

    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const idlFile = process.env.IDL_FILE || "target/idl/anchor_crash.json";
    const idl = require(require("path").join(process.cwd(), idlFile));
    const PROGRAM_ID = new PublicKey(process.env.PROGRAM_ID);
    const program = new anchor.Program(idl, PROGRAM_ID, provider);

    const payer = provider.wallet.publicKey;
    console.log("Program ID :", PROGRAM_ID.toBase58());
    console.log("Payer     :", payer.toBase58());

    // PDAs
    const [vaultPda] = PublicKey.findProgramAddressSync([Buffer.from("vault")], PROGRAM_ID);
    const [adminPda] = PublicKey.findProgramAddressSync([Buffer.from("admin")], PROGRAM_ID);
    console.log("Vault PDA :", vaultPda.toBase58());
    console.log("Admin PDA :", adminPda.toBase58());

    // Try combined tx first
    try {
      const tx = await program.methods
        .initVault()
        .accounts({
          payer,
          vault: vaultPda,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([])
        .postInstructions([
          await program.methods
            .initAdmin([...anchor.utils.bytes.bs58.decode(process.env.ADMIN_PUBKEY_BASE58)])
            .accounts({
              authority: payer,
              adminConfig: adminPda,
              systemProgram: SystemProgram.programId,
            })
            .instruction(),
        ])
        .rpc({ skipPreflight: false });
      console.log("✅ init_vault + init_admin tx:", tx);
    } catch (err) {
      console.warn("Combined tx failed, retrying stepwise…", err?.message || err);

      // init_vault
      try {
        const tx1 = await program.methods
          .initVault()
          .accounts({
            payer,
            vault: vaultPda,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
        console.log("init_vault:", tx1);
      } catch (e) {
        console.warn("init_vault skipped/failed:", e?.message || e);
      }

      // init_admin
      try {
        const tx2 = await program.methods
          .initAdmin([...anchor.utils.bytes.bs58.decode(process.env.ADMIN_PUBKEY_BASE58)])
          .accounts({
            authority: payer,
            adminConfig: adminPda,
            systemProgram: SystemProgram.programId,
          })
          .rpc();
        console.log("init_admin:", tx2);
      } catch (e) {
        console.warn("init_admin skipped/failed:", e?.message || e);
      }
    }

    console.log("Done.");
  } catch (e) {
    console.error("Init failed:", e);
    process.exit(1);
  }
})();
