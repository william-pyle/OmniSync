import {test,expect,type Page} from '@playwright/test';

// This is an explicit IPC fixture, not a simulated production mode. Native and
// cloud verification are recorded separately in docs/verification.json.
async function fixture(page:Page,phase='idle',theme='dark'){
  await page.addInitScript(({phase,theme})=>{
    const callbacks=new Map<number,Function>();let next=0;(window as any).isTauri=true;
    const config={revision:1,deviceId:'test-device',roots:[],rules:[],pausedUntil:phase==='paused'?0:null,continuous:true,pauseOnMetered:true,priorVersions:10,stagingBudgetBytes:50*2**30,freeSpaceReserveBytes:5*2**30,uploadLimitKib:null,startAtSignIn:false,theme,destination:{path:'D:\\Backups',id:'fixture-destination'}};
    const snapshot={config,account:{connected:true,displayName:'Test account',email:'test@example.invalid',quotaUsed:0,quotaLimit:0},progress:{phase,currentFile:'Sample/file.txt',sent:10,total:100,message:'Fixture state',lastBackup:null,connecting:false},counts:{backedUp:0,pending:0,excluded:0,issues:0,retained:0},recent:[],oauthConfigured:true,version:'test'};
    (window as any).__fixture={snapshot,previewDelay:0,update:null,downloadCalls:0,installCalls:0,lastConfig:null};
    (window as any).__TAURI_INTERNALS__={transformCallback:(fn:Function)=>{const id=next++;callbacks.set(id,fn);return id;},unregisterCallback:(id:number)=>callbacks.delete(id),invoke:async(name:string,args:any)=>{
      if(name==='plugin:event|listen')return 1;if(name==='plugin:event|unlisten')return;
      if(name!=='command')throw Error('Unexpected invocation');
      if(['preview','shellPreview','undo'].includes(args.action)&& (window as any).__fixture.previewDelay)await new Promise(resolve=>setTimeout(resolve,(window as any).__fixture.previewDelay));
      switch(args.action){case 'checkUpdate':return (window as any).__fixture.update;case 'downloadUpdate':(window as any).__fixture.downloadCalls++;return;case 'installUpdate':(window as any).__fixture.installCalls++;return;case 'pickDestination':return 'X:\\Backup folder';case 'snapshot':return structuredClone(snapshot);case 'intent':return null;case 'pause':config.pausedUntil=args.payload.seconds===0?0:Date.now()/1000+args.payload.seconds;snapshot.progress.phase='paused';return;case 'resume':config.pausedUntil=null;snapshot.progress.phase='idle';return;case 'syncNow':if(config.pausedUntil!==null)throw Error('Resume first');return;case 'inventory':return [];case 'items':return [];case 'preview':(window as any).__fixture.lastConfig=structuredClone(args.payload.config);return {token:'fixture',revision:config.revision,description:args.payload.description,included:3,excluded:1,bytes:1024,examples:['Example — included'],warnings:[]};case 'apply':Object.assign(config,(window as any).__fixture.lastConfig??config);config.revision++;return;default:throw Error(`Unimplemented test command: ${args.action}`);}
    }};
  },{phase,theme});
  await page.goto('/');await expect(page.getByRole('heading',{name:'Your files, protected.'})).toBeVisible();
}
test('plain browser never invents engine success',async({page})=>{await page.goto('/');await expect(page.getByRole('heading',{name:'Open the OmniSync desktop app'})).toBeVisible();});
test('pause changes control state and resume restores sync',async({page})=>{await fixture(page);await page.getByRole('button',{name:'Pause',exact:true}).click();await page.locator('#pauseDuration').selectOption('0');await page.getByRole('button',{name:'Pause backup',exact:true}).click();await expect(page.getByRole('heading',{name:'Backup paused'})).toBeVisible();await expect(page.getByRole('button',{name:'Sync now',exact:true})).toBeDisabled();await page.getByRole('button',{name:'Resume',exact:true}).click();await expect(page.getByRole('button',{name:'Sync now',exact:true})).toBeEnabled();});
test('rules require preview before applying',async({page})=>{await fixture(page);await page.getByRole('button',{name:'Rules',exact:true}).click();await page.getByRole('button',{name:'Add rule',exact:true}).click();await page.locator('#ruleValue').fill('*.tmp');await page.getByRole('button',{name:'Preview impact'}).click();await expect(page.getByRole('heading',{name:'Review changes'})).toBeVisible();await expect(page.getByText('3 included',{exact:true})).toBeVisible();await page.getByRole('button',{name:'Apply changes'}).click();await expect(page.getByRole('dialog')).toHaveCount(0);});
test('syncing Mobius visibly advances and reduced motion is static',async({page})=>{await page.emulateMedia({reducedMotion:'no-preference'});await fixture(page,'syncing');const mark=page.locator('.orbit .mark');const before=await mark.evaluate(el=>getComputedStyle(el).transform);await page.waitForTimeout(350);const after=await mark.evaluate(el=>getComputedStyle(el).transform);expect(after).not.toBe(before);await page.emulateMedia({reducedMotion:'reduce'});await expect.poll(()=>mark.evaluate(el=>getComputedStyle(el).animationName)).toBe('none');});
for(const width of [390,768,1440])for(const theme of ['light','dark'])test(`${theme} navigation stays within ${width}px`,async({page})=>{await page.setViewportSize({width,height:950});await fixture(page,'idle',theme);for(const name of ['Folders','Rules','Activity','Restore','Explorer','Settings','Overview']){await page.getByRole('button',{name,exact:true}).click();expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);await expect(page.locator('main h1')).toBeVisible();}await page.screenshot({path:`test-results/${theme}-${width}.png`,fullPage:true});});


test('Overview metrics update through polling without navigation',async({page})=>{await fixture(page,'syncing');const count=page.locator('.metric').filter({hasText:'Backed up'}).locator('strong');await expect(count).toHaveText('0');await page.evaluate(()=>{const s=(window as any).__fixture.snapshot;s.counts.backedUp=42;s.counts.pending=3;});await expect(count).toHaveText('42',{timeout:4000});await expect(page.getByRole('heading',{name:'Your files, protected.'})).toBeVisible();});
test('review opens immediately while calculating and canceled results stay closed',async({page})=>{await fixture(page);await page.evaluate(()=>(window as any).__fixture.previewDelay=1800);await page.getByRole('button',{name:'Rules',exact:true}).click();await page.getByRole('button',{name:'Add rule',exact:true}).click();await page.locator('#ruleValue').fill('*.tmp');await page.getByRole('button',{name:'Preview impact'}).click();await expect(page.getByRole('heading',{name:'Review changes'})).toBeVisible({timeout:500});await expect(page.getByRole('button',{name:'Apply changes'})).toBeDisabled();await expect(page.getByText('Calculating affected files...')).toBeVisible();await page.getByRole('button',{name:'Cancel',exact:true}).click();await page.waitForTimeout(2100);await expect(page.getByRole('dialog')).toHaveCount(0);});

test('destination review keeps all source selections and exclusions',async({page})=>{
  await fixture(page);
  await page.evaluate(()=>{const c=(window as any).__fixture.snapshot.config;c.roots=[{id:'saved-root',label:'Saved folder',path:'C:\\Source',volumeIdentity:'original',enabled:true,isFile:false}];c.rules=[{id:'saved-rule',scope:'saved-root',kind:'exactFile',action:'exclude',value:'private.txt',order:3}];});
  await expect(page.getByText('Saved folder',{exact:true})).toBeVisible({timeout:4000});
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  await page.getByRole('button',{name:'Change destination',exact:true}).click();
  await page.locator('#destinationPath').fill('\\\\server\\share\\Backups');
  await page.getByRole('button',{name:'Review destination',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Review changes'})).toBeVisible();
  const c=await page.evaluate(()=>(window as any).__fixture.lastConfig);
  expect(c.roots[0].id).toBe('saved-root');expect(c.roots[0].volumeIdentity).toBe('original');
  expect(c.rules[0]).toEqual({id:'saved-rule',scope:'saved-root',kind:'exactFile',action:'exclude',value:'private.txt',order:3});
  expect(c.destination.path).toBe('\\\\server\\share\\Backups');
  await page.getByRole('button',{name:'Apply changes'}).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
});
test('upgrade without destination explains preservation and disables sync',async({page})=>{
  await fixture(page);await page.evaluate(()=>{const c=(window as any).__fixture.snapshot.config;c.destination=null;c.roots=[{id:'saved',label:'Saved',path:'C:/Source',enabled:true}];});
  await expect(page.getByRole('button',{name:'Choose destination',exact:true})).toBeVisible({timeout:4000});
  await expect(page.getByRole('button',{name:'Sync now',exact:true})).toBeDisabled();
  await expect(page.getByText(/existing folder selections and exclusions are preserved/)).toBeVisible();
  await expect(page.getByRole('button',{name:/OAuth|Connect Google/})).toHaveCount(0);
});
test('update download and install each need an explicit choice',async({page})=>{
  await fixture(page);await page.evaluate(()=>(window as any).__fixture.update={version:'0.3.0',notes:'New release <safe text>',size:2048});
  await page.getByRole('button',{name:'Settings',exact:true}).click();
  await page.getByRole('button',{name:'Check for updates',exact:true}).click();
  await expect(page.getByRole('heading',{name:'OmniSync 0.3.0 is available'})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__fixture.downloadCalls)).toBe(0);
  await page.getByRole('button',{name:'Download update',exact:true}).click();
  await expect(page.getByRole('button',{name:'Close OmniSync & install',exact:true})).toBeVisible();
  expect(await page.evaluate(()=>(window as any).__fixture.installCalls)).toBe(0);
  await page.getByRole('button',{name:'Close OmniSync & install',exact:true}).click();
  expect(await page.evaluate(()=>(window as any).__fixture.installCalls)).toBe(1);
});
