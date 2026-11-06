// Test-only oracle. Regenerate with @solana/kit exactly8.4.0; all signing seeds
// below are public fixture constants and must never hold real funds.
const fs=require('node:fs');
const kit=require(process.env.SOLANA_KIT_MODULE||'@solana/kit');
const {createPrivateKey,sign}=require('node:crypto');
const hex=b=>Buffer.from(b).toString('hex');
const address=b=>kit.getAddressDecoder().decode(b);
const bytes=a=>kit.getAddressEncoder().encode(a);
const key=i=>new Uint8Array(32).fill(i);
const token=kit.address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const mint=kit.address('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');
const system=kit.address('11111111111111111111111111111111');
const compute=kit.address('ComputeBudget111111111111111111111111111111');
const associated=kit.address('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
const ref=address(key(3));const deposit=address(key(2));
const ata=async owner=>(await kit.getProgramDerivedAddress({programAddress:associated,seeds:[bytes(owner),bytes(token),bytes(mint)]}))[0];
async function fixture(name,version,asset,amount,options={}){
 const signer=await kit.createKeyPairSignerFromPrivateKeyBytes(key(1));const source=asset===0?await ata(signer.address):signer.address;const intendedDestination=asset===0?await ata(deposit):deposit;const destination=options.wrongDestination?address(key(9)):intendedDestination;
 const data=new Uint8Array(asset===0?10:12);const view=new DataView(data.buffer);
 if(asset===0){data[0]=12;view.setBigUint64(1,amount,true);data[9]=6;}else{view.setUint32(0,2,true);view.setBigUint64(4,amount,true);}
 const account=(address,role)=>({address,role});
 const accounts=asset===0?[account(source,1),account(options.wrongMint?address(key(8)):mint,0),account(destination,1),account(signer.address,3),account(ref,options.writableRef?1:0)]:[account(signer.address,3),account(destination,1),account(ref,options.writableRef?1:0)];
 let m=kit.createTransactionMessage({version});m=kit.setTransactionMessageFeePayerSigner(signer,m);m=kit.setTransactionMessageLifetimeUsingBlockhash({blockhash:address(key(4)),lastValidBlockHeight:123n},m);
 const prefix=options.compute?[{programAddress:compute,accounts:[],data:Uint8Array.of(2,64,66,15,0)},{programAddress:compute,accounts:[],data:Uint8Array.of(3,1,0,0,0,0,0,0,0)}]:[];
 if(options.omitRef)accounts.pop();
 if(options.secondSigner){const extra=await kit.createKeyPairSignerFromPrivateKeyBytes(key(7));accounts[accounts.length-1]={address:extra.address,role:2,signer:extra};}
 const payment={programAddress:options.wrongProgram?address(key(8)):(asset===0?token:system),accounts,data};
 const setup=options.ata?[{programAddress:associated,accounts:[account(signer.address,3),account(destination,1),account(deposit,0),account(mint,0),account(system,0),account(token,0)],data:Uint8Array.of(1)}]:[];
 m=kit.appendTransactionMessageInstructions([...prefix,...setup,payment,...(options.extraTransfer?[payment]:[])],m);
 const tx=await kit.signTransactionMessageWithSigners(m);const wire=kit.getTransactionEncoder().encode(tx);const decoded=kit.getCompiledTransactionMessageDecoder().decode(tx.messageBytes);
 // Independently verify that the SDK signature is the OpenSSL Ed25519 result.
 const privateKey=createPrivateKey({key:Buffer.concat([Buffer.from('302e020100300506032b657004220420','hex'),Buffer.from(key(1))]),format:'der',type:'pkcs8'});
 const signature=tx.signatures[signer.address];if(!Buffer.from(signature).equals(sign(null,tx.messageBytes,privateKey)))throw Error('Ed25519 oracle mismatch');
 const pre=decoded.staticAccounts.map(()=>0n),post=decoded.staticAccounts.map(()=>0n);const payerIndex=decoded.staticAccounts.indexOf(signer.address),sourceIndex=decoded.staticAccounts.indexOf(source),destIndex=decoded.staticAccounts.indexOf(destination);
 pre[payerIndex]=1000000000000000000n;post[payerIndex]=pre[payerIndex]-5000n-(asset===1?amount:0n);if(asset===1)post[destIndex]=amount;
 let preTokens=[],postTokens=[];
 if(asset===0){pre[sourceIndex]=post[sourceIndex]=2039280n;pre[destIndex]=post[destIndex]=2039280n;const balance=(index,owner,amount)=>({accountIndex:index,mint,owner,programId:token,uiTokenAmount:{amount:amount.toString(),decimals:6,uiAmount:null,uiAmountString:'0'}});preTokens=[balance(sourceIndex,signer.address,1000000000000000000n),balance(destIndex,deposit,0n)];postTokens=[balance(sourceIndex,signer.address,1000000000000000000n-amount),balance(destIndex,deposit,amount)];}
 const response={jsonrpc:'2.0',id:41,result:{slot:'18446744073709551615',blockTime:1001,version,transaction:[Buffer.from(wire).toString('base64'),'base64'],meta:{err:null,fee:5000,loadedAddresses:{writable:[],readonly:[]},innerInstructions:[],preBalances:pre.map(String),postBalances:post.map(String),preTokenBalances:preTokens,postTokenBalances:postTokens}}};
 // Decimal-string sentinels are expanded only for numeric RPC fields, avoiding
 // JavaScript Number truncation above 2^53. Token amount remains a JSON string.
 let raw=JSON.stringify(response).replace('"slot":"18446744073709551615"','"slot":18446744073709551615');for(const name of ['preBalances','postBalances'])raw=raw.replace(new RegExp('"'+name+'":\\[(.*?)\\]'),(_,items)=>'"'+name+'":['+items.replace(/"([0-9]+)"/g,'$1')+']');
 return {name,version,asset,amount:amount.toString(),quote_amount:(options.quoteAmount??amount).toString(),expected_payment:options.reject!==true,signature:kit.getSignatureFromTransaction(tx),signature_hex:hex(signature),wire_hex:hex(wire),message_hex:hex(tx.messageBytes),payer:signer.address,payer_hex:hex(bytes(signer.address)),deposit:deposit,deposit_hex:hex(bytes(deposit)),reference:ref,reference_hex:hex(bytes(ref)),mint:mint,mint_hex:hex(bytes(mint)),source,destination,destination_hex:hex(bytes(intendedDestination)),account_keys:decoded.staticAccounts,instruction_count:decoded.instructions.length,signature_count:Object.keys(tx.signatures).length,slot_text:'18446744073709551615',response:raw};
}
(async()=>{const fixtures=[await fixture('sol-legacy','legacy',1,5000000n),await fixture('usdc-v0',0,0,5000000n,{compute:true}),await fixture('sol-wide-v0',0,1,9007199254740993n),await fixture('sol-under','legacy',1,4999999n,{quoteAmount:5000000n,reject:true}),await fixture('sol-one-percent','legacy',1,5050000n,{quoteAmount:5000000n}),await fixture('sol-over','legacy',1,5050001n,{quoteAmount:5000000n,reject:true}),await fixture('sol-wrong-recipient','legacy',1,5000000n,{wrongDestination:true,reject:true}),await fixture('usdc-wrong-mint',0,0,5000000n,{wrongMint:true,reject:true}),await fixture('usdc-wrong-program',0,0,5000000n,{wrongProgram:true,reject:true}),await fixture('sol-omitted-reference','legacy',1,5000000n,{omitRef:true,reject:true}),await fixture('sol-writable-reference','legacy',1,5000000n,{writableRef:true,reject:true}),await fixture('sol-extra-transfer','legacy',1,5000000n,{extraTransfer:true,reject:true}),await fixture('two-signatures','legacy',1,5000000n,{secondSigner:true,reject:true}),await fixture('usdc-ata-idempotent',0,0,5000000n,{ata:true,compute:true})];fs.writeFileSync(process.argv[2],JSON.stringify({schema_version:1,sdk:'@solana/kit8.4.0',independent_signature:'node:crypto/OpenSSL Ed25519',fixtures},null,2)+'\n');})();
