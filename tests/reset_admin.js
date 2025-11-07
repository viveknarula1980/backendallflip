const anchor = require("@coral-xyz/anchor");
const bs58 = require("bs58");

(async () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Casino;

  const adminPubkey = bs58.decode(
    process.env.ADMIN_PUBKEY_BASE58 ||
      "EWaMqbKeyv2V2WheLUDuuFs7DqTqhARVd34JvkZPRu7z"
  );

  const [adminConfig] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("admin")],
    program.programId
  );

  const tx = await program.methods
    .initAdmin([...adminPubkey]) // 32-byte array
    .accounts({
      authority: provider.wallet.publicKey,
      adminConfig,
      systemProgram: anchor.web3.SystemProgram.programId,
    })
    .rpc();

  console.log("✅ Admin reset success tx:", tx);
})();
