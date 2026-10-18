const fs=require('fs'),path=require('path');
const sharp=process.env.AXENTRA_SHARP_PATH?require(process.env.AXENTRA_SHARP_PATH):require('sharp');
const root=__dirname,svg=fs.readFileSync(path.join(root,'axentra-usb.svg'),'utf8');
function dib(raw,n){
 const maskRow=Math.ceil(n/32)*4,out=Buffer.alloc(40+n*n*4+maskRow*n);
 out.writeUInt32LE(40,0);out.writeInt32LE(n,4);out.writeInt32LE(n*2,8);out.writeUInt16LE(1,12);out.writeUInt16LE(32,14);out.writeUInt32LE(n*n*4+maskRow*n,20);
 for(let y=0;y<n;y++)for(let x=0;x<n;x++){
  const from=(y*n+x)*4,to=40+((n-1-y)*n+x)*4;
  out[to]=raw[from+2];out[to+1]=raw[from+1];out[to+2]=raw[from];out[to+3]=raw[from+3];
 }
 return out;
}
(async()=>{
 const entries=[];
 for(const n of [16,24,32,48,64,128,256,512,1024]){
  const source=n<=24?svg.replace('stroke-width="1.75"','stroke-width="2.2"').replace('<path d="M29 45h6" opacity=".75"/>',''):svg;
  const png=await sharp(Buffer.from(source),{density:384}).resize(n,n).png().toBuffer();
  fs.writeFileSync(path.join(root,`axentra-usb-${n}.png`),png);
  if(n<=256)entries.push({n,bytes:n===256?png:dib(await sharp(png).ensureAlpha().raw().toBuffer(),n)});
 }
 const head=Buffer.alloc(6+entries.length*16);head.writeUInt16LE(1,2);head.writeUInt16LE(entries.length,4);let offset=head.length;
 entries.forEach(({n,bytes},i)=>{const p=6+i*16;head[p]=head[p+1]=n===256?0:n;head.writeUInt16LE(1,p+4);head.writeUInt16LE(32,p+6);head.writeUInt32LE(bytes.length,p+8);head.writeUInt32LE(offset,p+12);offset+=bytes.length;});
 fs.writeFileSync(path.join(root,'axentra-usb.ico'),Buffer.concat([head,...entries.map(e=>e.bytes)]));
 console.log('Updated SVG-derived PNG and Windows ICO assets.');
})().catch(e=>{console.error(e);process.exit(1)});
