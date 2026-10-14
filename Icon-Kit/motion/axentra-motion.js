/* Axentra Glass + Precision motion. Original WebGL geometry; no runtime dependencies. */
(() => {
  'use strict';
  const TAU = Math.PI * 2;
  const APPS = [
    ['command', 'Command', '#62B4FF'], ['ghost', 'Ghost', '#AA7EFF'],
    ['vault', 'Vault', '#408BFF'], ['crystalwire', 'CrystalWire', '#1FD0EA'],
    ['horizon', 'Horizon', '#FFB446'], ['pdf', 'PDF', '#FF6977'],
    ['spaces', 'Spaces', '#1AD3C0'], ['mail', 'Mail', '#EF88C7']
  ];
  const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
  const smooth = x => { x = clamp(x, 0, 1); return x*x*(3-2*x); };
  function spring(s, target, dt, stiffness = 115, damping = 11) {
    const steps = Math.max(1, Math.ceil(dt / (1/120)));
    for (let j = 0; j < steps; j++) {
      const h = dt / steps;
      s.v += ((target-s.x)*stiffness - s.v*damping)*h;
      s.x += s.v*h;
    }
  }
  const VERTEX = `
    attribute vec2 aPoint;
    uniform vec2 uViewport;
    uniform vec3 uCenter;
    uniform vec3 uRotation;
    uniform float uSize, uBend, uTwist, uFocal;
    varying vec2 vUv;
    varying vec3 vNormal;
    mat3 rotate(vec3 r) {
      float cx=cos(r.x), sx=sin(r.x), cy=cos(r.y), sy=sin(r.y), cz=cos(r.z), sz=sin(r.z);
      mat3 rx=mat3(1.,0.,0., 0.,cx,sx, 0.,-sx,cx);
      mat3 ry=mat3(cy,0.,-sy, 0.,1.,0., sy,0.,cy);
      mat3 rz=mat3(cz,sz,0., -sz,cz,0., 0.,0.,1.);
      return rz*ry*rx;
    }
    void main() {
      vec2 p=aPoint;
      float z=uBend*(p.x*p.x-.12) + uTwist*p.x*p.y;
      vec3 normal=normalize(vec3(-2.*uBend*p.x-uTwist*p.y,-uTwist*p.x,1.));
      mat3 r=rotate(uRotation);
      vec3 world=r*vec3(p,z)*uSize+uCenter;
      float w=uFocal-world.z;
      // A real perspective depth value prevents curled portions painting over
      // nearer parts of the same tile when its silhouette folds during a turn.
      float depth=(5010./4990.)*w-(100000./4990.);
      gl_Position=vec4(world.x*uFocal*2./uViewport.x,world.y*uFocal*2./uViewport.y,depth,w);
      vUv=vec2(p.x+.5,.5-p.y);
      vNormal=r*normal;
    }`;
  const FRAGMENT = `
    precision mediump float;
    uniform sampler2D uImage;
    uniform float uHover;
    varying vec2 vUv;
    varying vec3 vNormal;
    void main() {
      vec4 tex=texture2D(uImage,vUv);
      if(tex.a<.003) discard;
      vec3 n=normalize(vNormal);
      float light=.84+.16*abs(n.z);
      float sheen=pow(max(0.,dot(n,normalize(vec3(-.38,.65,1.)))),22.);
      gl_FragColor=vec4(tex.rgb*light+vec3(.12,.16,.21)*sheen*(.28+uHover*.28),tex.a);
    }`;

  function makeGL(canvas) {
    const gl = canvas.getContext('webgl', {alpha:true, antialias:true, premultipliedAlpha:false});
    if (!gl) throw new Error('WebGL is unavailable');
    function compile(type, text) {
      const s=gl.createShader(type); gl.shaderSource(s,text); gl.compileShader(s);
      if (!gl.getShaderParameter(s,gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      return s;
    }
    const vs=compile(gl.VERTEX_SHADER,VERTEX), fs=compile(gl.FRAGMENT_SHADER,FRAGMENT);
    const program=gl.createProgram(); gl.attachShader(program,vs); gl.attachShader(program,fs); gl.linkProgram(program);
    gl.deleteShader(vs); gl.deleteShader(fs);
    if (!gl.getProgramParameter(program,gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
    gl.useProgram(program);
    const vertices=[], indices=[], n=32;
    for (let y=0;y<=n;y++) for(let x=0;x<=n;x++) vertices.push(x/n-.5,y/n-.5);
    for (let y=0;y<n;y++) for(let x=0;x<n;x++) {
      const a=y*(n+1)+x,b=a+1,c=a+n+1,d=c+1;
      indices.push(a,b,c,b,d,c);
    }
    const vb=gl.createBuffer(), ib=gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER,vb); gl.bufferData(gl.ARRAY_BUFFER,new Float32Array(vertices),gl.STATIC_DRAW);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,ib); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,new Uint16Array(indices),gl.STATIC_DRAW);
    const location=gl.getAttribLocation(program,'aPoint');
    gl.enableVertexAttribArray(location); gl.vertexAttribPointer(location,2,gl.FLOAT,false,0,0);
    // Back faces use the same UVs: reverse views are naturally mirrored, never blank.
    gl.disable(gl.CULL_FACE); gl.enable(gl.DEPTH_TEST); gl.depthFunc(gl.LEQUAL);
    gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA,gl.ONE_MINUS_SRC_ALPHA);
    const uniforms={};
    for (const name of ['Viewport','Center','Rotation','Size','Bend','Twist','Focal','Image','Hover'])
      uniforms[name]=gl.getUniformLocation(program,'u'+name);
    gl.uniform1i(uniforms.Image,0);
    return {gl,program,uniforms,count:indices.length,vb,ib};
  }

  async function mount(host, options={}) {
    if (!host) throw new Error('A motion host element is required');
    const canvas=document.createElement('canvas');
    canvas.className='axm-canvas'; canvas.setAttribute('aria-hidden','true');
    const interaction=document.createElement('div'); interaction.className='axm-interaction';
    host.append(canvas,interaction);
    const media=matchMedia('(prefers-reduced-motion: reduce)');
    let reduced=media.matches, paused=options.autoplay===false, disposed=false, visible=true;
    let time=0, frame=0, previous=performance.now(), width=1,height=1, dirty=true;
    let device, failure=null, selected=null;
    const removers=[];
    function listen(el,name,fn,opts) { el.addEventListener(name,fn,opts); removers.push(()=>el.removeEventListener(name,fn,opts)); }
    try { device=makeGL(canvas); } catch(error) { failure=error; canvas.hidden=true; }
    const items=APPS.map(([id,name,color],i)=>{
      const button=document.createElement('button');
      button.type='button'; button.className='axm-target'; button.dataset.app=id;
      button.setAttribute('aria-label',`Spin Axentra ${name}`);
      button.style.setProperty('--product',color);
      const image=new Image(); image.alt=''; image.className='axm-fallback';
      image.src=options.images?.[id] || `${options.assetBase || '../assets'}/${id}-256.png`;
      const label=document.createElement('span'); label.className='axm-label'; label.textContent=name;
      button.append(image,label); interaction.append(button);
      const item={id,name,color,i,button,image,texture:null,hover:false,focused:false,boost:{x:0,v:0},
        spin:{x:0,v:0},spinTarget:0,tilt:{x:0,y:0},pointer:{x:0,y:0},pose:null};
      listen(button,'pointerenter',()=>{item.hover=true;if(!reduced)item.boost.v+=2.8;});
      listen(button,'pointerleave',()=>{item.hover=false;item.pointer={x:0,y:0};});
      listen(button,'pointermove',e=>{
        const r=button.getBoundingClientRect();
        item.pointer={x:clamp((e.clientX-r.left)/r.width-.5,-.5,.5),y:clamp((e.clientY-r.top)/r.height-.5,-.5,.5)};
      });
      listen(button,'focus',()=>{item.focused=true;});
      listen(button,'blur',()=>{item.focused=false;});
      listen(button,'click',()=>{
        selected=id;
        if(!reduced){item.spinTarget+=TAU;item.boost.v+=5;}
        options.onSelect?.({id,name});
      });
      return item;
    });
    const loaded=await Promise.allSettled(items.map(async item=>{
      await item.image.decode();
      if(!device || disposed)return;
      const {gl}=device, texture=gl.createTexture(); item.texture=texture;
      gl.bindTexture(gl.TEXTURE_2D,texture);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_S,gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_WRAP_T,gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.LINEAR_MIPMAP_LINEAR);
      gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.LINEAR);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL,false);
      gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA,gl.RGBA,gl.UNSIGNED_BYTE,item.image);
      gl.generateMipmap(gl.TEXTURE_2D);
    }));
    const broken=loaded.find(x=>x.status==='rejected');
    if(broken){failure=broken.reason;canvas.hidden=true;device=null;}
    host.classList.toggle('axm-ready',!!device);
    host.dataset.motion=device?'webgl':'static';
    if(failure)options.onError?.(failure);
    function resize(){
      width=host.clientWidth || 1; height=host.clientHeight || 1;
      const dpr=Math.min(devicePixelRatio || 1,2);
      canvas.width=Math.round(width*dpr); canvas.height=Math.round(height*dpr);
      device?.gl.viewport(0,0,canvas.width,canvas.height);dirty=true;
    }
    const ro=new ResizeObserver(resize);ro.observe(host);resize();
    const io=new IntersectionObserver(entries=>{visible=entries[0].isIntersecting;});io.observe(host);
    listen(media,'change',e=>{reduced=e.matches; previous=performance.now();dirty=true;});
    function pose(item,dt){
      const active=item.hover || item.focused;
      const still=reduced || !device;
      if(still){item.boost.x=0;item.boost.v=0;item.spin.x=0;item.spin.v=0;item.spinTarget=0;}
      else {spring(item.boost,active?1:0,dt);spring(item.spin,item.spinTarget,dt,27,9.2);}
      const lerp=1-Math.exp(-9*dt);
      item.tilt.x+=(item.pointer.x-item.tilt.x)*lerp;
      item.tilt.y+=(item.pointer.y-item.tilt.y)*lerp;
      const mobile=width<660, t=still?0:time, i=item.i;
      const initial=-Math.PI/2+i*TAU/8;
      const a=initial+t*.045;
      let x,y,z,size;
      if(mobile){
        x=(i%2 ? .225 : -.225)*width;
        y=height/2-190-Math.floor(i/2)*156;
        z=0;size=Math.min(100,width*.25);
      } else {
        x=Math.cos(a)*width*.365;
        y=Math.sin(a)*height*.362;
        z=Math.sin(a+.4)*50;
        size=clamp(width*.103,92,151)*(1+.055*Math.cos(i*1.7));
      }
      // Each tile has a different curve/twist phase, never an independent flat rotate.
      const flipPhase=((t+i*2.1)%32)/32;
      const turn=still?0:smooth((flipPhase-.55)/.32)*TAU;
      const yaw=still?0:turn+Math.sin(t*.32+i*.7)*.23+item.spin.x+item.tilt.x*.48;
      const bend=still?0:Math.sin(yaw)*(.9+(i%3)*.16)+Math.sin(t*.65+i)*.12+item.boost.x*.24;
      const twist=still?0:Math.sin(yaw+i*.9)*.24+item.tilt.y*.30;
      const rx=still?0:Math.sin(t*.28+i)*.10-item.tilt.y*.36;
      const rz=still?0:Math.sin(t*.21+i*.8)*.055;
      size*=1+item.boost.x*.17; y+=item.boost.x*17;z+=item.boost.x*65;
      const focal=Math.max(width,height)*1.35;
      const scale=focal/(focal-z);
      const sx=width/2+x*scale,sy=height/2-y*scale;
      const hit=Math.max(52,size*scale*.96);
      item.button.style.width=`${hit}px`;item.button.style.height=`${hit}px`;
      item.button.style.transform=`translate(${sx-hit/2}px,${sy-hit/2}px)`;
      item.button.style.zIndex=String(Math.round(z+200));
      item.pose={x,y,z,size,focal,rx,yaw,rz,bend,twist,sx,sy,hit,hover:item.boost.x};
      return item.pose;
    }
    function draw(dt){
      for(const item of items)pose(item,dt);
      if(!device)return;
      const {gl,uniforms:u}=device;
      gl.clearColor(0,0,0,0);gl.clear(gl.COLOR_BUFFER_BIT);
      gl.uniform2f(u.Viewport,width,height);
      for(const item of [...items].sort((a,b)=>a.pose.z-b.pose.z)){
        gl.clear(gl.DEPTH_BUFFER_BIT);
        const p=item.pose;
        gl.uniform3f(u.Center,p.x,p.y,p.z);gl.uniform3f(u.Rotation,p.rx,p.yaw,p.rz);
        gl.uniform1f(u.Size,p.size);gl.uniform1f(u.Bend,p.bend);gl.uniform1f(u.Twist,p.twist);
        gl.uniform1f(u.Focal,p.focal);gl.uniform1f(u.Hover,p.hover);
        gl.bindTexture(gl.TEXTURE_2D,item.texture);
        gl.drawElements(gl.TRIANGLES,device.count,gl.UNSIGNED_SHORT,0);
      }
    }
    function tick(now){
      if(disposed)return;
      const dt=clamp((now-previous)/1000,0,.04);previous=now;
      if(visible && !document.hidden){
        if(!paused&&!reduced)time+=dt;
        if((device&&!reduced)||dirty){draw(dt);dirty=false;}
      }
      frame=requestAnimationFrame(tick);
    }
    listen(canvas,'webglcontextlost',e=>{
      e.preventDefault();canvas.hidden=true;device=null;
      host.classList.remove('axm-ready');host.dataset.motion='static';dirty=true;
    });
    draw(0);frame=requestAnimationFrame(tick);
    return {
      setPaused(value){paused=!!value;},
      getState(){return {time,paused,reduced,renderer:device?'webgl':'static',selected,
        items:items.map(i=>({id:i.id,loaded:i.image.complete&&i.image.naturalWidth>0,...i.pose}))};},
      // Deterministic visual QA and still-image export; not required by the demo.
      seek(seconds){time=Math.max(0,Number(seconds)||0);draw(0);},
      destroy(){
        disposed=true;cancelAnimationFrame(frame);ro.disconnect();io.disconnect();removers.forEach(f=>f());
        if(device){for(const i of items)device.gl.deleteTexture(i.texture);device.gl.deleteBuffer(device.vb);device.gl.deleteBuffer(device.ib);device.gl.deleteProgram(device.program);}
        canvas.remove();interaction.remove();host.classList.remove('axm-ready');delete host.dataset.motion;
      }
    };
  }
  window.AxentraMotion={mount,apps:APPS.map(([id,name,color])=>({id,name,color}))};
})();
