<script lang="ts">
  import LevelMeter from './LevelMeter.svelte';
  import Segmented from './Segmented.svelte';
  import Toggle from './Toggle.svelte';
  import { t } from './i18n';
  import { store } from './store.svelte';
  import type { Mode } from './types';

  let s = $derived(store.state!);
  let step = $state(0);
  const modes: Mode[] = ['call', 'streaming', 'recording'];
  let mics = $derived(s.settings.show_all_devices ? [...s.devices, ...s.hidden_devices] : s.devices);

  // Level check: listen while the user talks, then set the input level so that loud syllables
  // peak around -6 dBFS. The device volume is on a cubic scale: amplitude ~ volume^3, so a
  // change of x dB needs the volume multiplied by 10^(x/60).
  const TARGET_PEAK_DB = -6;
  let measuring = $state(false);
  let peak = $state(-120);
  let result = $state<'' | 'set' | 'quiet' | 'none'>('');
  async function measure() {
    result = '';
    measuring = true;
    peak = -120;
    const until = Date.now() + 6000;
    while (Date.now() < until) {
      peak = Math.max(peak, store.levels.input_db);
      await new Promise((r) => setTimeout(r, 50));
    }
    measuring = false;
    const current = s.mic_volume;
    if (current === null) return (result = 'none');
    if (peak < -45) return (result = 'quiet');
    const next = Math.min(1, Math.max(0.3, current * Math.pow(10, (TARGET_PEAK_DB - peak) / 60)));
    await store.apply({ mic_level: { volume: Math.round(next * 100) / 100 } });
    result = 'set';
  }

  let copied = $state(false);
  async function copyName() {
    try {
      await navigator.clipboard.writeText('Sordino Mic');
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* the name is shown anyway */
    }
  }

  function finish() {
    store.apply({ onboarded: true });
  }
</script>

<div class="scrim" role="presentation"></div>
<div class="sheet" role="dialog" aria-modal="true" aria-label={t('onb.title')}>
  <header>
    <h2>{t('onb.title')}</h2>
    <span class="steps">{t('onb.step', { n: step + 1 })}</span>
  </header>

  {#if step === 0}
    <h3>{t('onb.mic')}</h3>
    <p class="hint">{t('onb.mic_hint')}</p>
    <select
      aria-label={t('mic.title')}
      value={s.settings.mic ?? ''}
      onchange={(e) => store.apply({ mic: e.currentTarget.value === '' ? null : e.currentTarget.value })}
    >
      <option value="">{t('mic.auto')}</option>
      {#each mics as d (d.id)}
        <option value={d.id}>{d.name}</option>
      {/each}
    </select>
    <LevelMeter label={t('meter.mic')} db={store.levels.input_db} active={s.status === 'running'} />
    <button class="secondary" disabled={measuring || s.mic_volume === null} onclick={measure}>
      {measuring ? t('onb.measuring') : t('onb.measure')}
    </button>
    {#if result === 'set'}<p class="ok">{t('onb.level_set', { v: Math.round((s.settings.mic_level.volume ?? 0) * 100) })}</p>{/if}
    {#if result === 'quiet'}<p class="warn">{t('onb.too_quiet')}</p>{/if}
    {#if result === 'none' || s.mic_volume === null}<p class="hint">{t('onb.no_level')}</p>{/if}
  {:else if step === 1}
    <h3>{t('onb.mode')}</h3>
    <p class="hint">{t('onb.mode_hint')}</p>
    <Segmented
      label={t('mode.title')}
      options={modes.map((m) => ({ value: m, label: t(`mode.${m}`) }))}
      value={s.settings.mode}
      onchange={(m) => store.apply({ mode: m })}
    />
    <p class="hint">{t(`mode.${s.settings.mode}_sub`)}</p>
    <div class="row">
      <div>
        <span>{t('onb.default')}</span>
        <small>{t('onb.default_sub')}</small>
      </div>
      <Toggle label={t('onb.default')} checked={s.settings.set_default} onchange={(v) => store.apply({ set_default: v })} />
    </div>
  {:else}
    <h3>{t('onb.apps')}</h3>
    <ol>
      <li>
        {t('onb.apps_1')}
        <button class="chip" onclick={copyName}>{copied ? t('onb.copied') : 'Sordino Mic'}</button>
      </li>
      <li>{t('onb.apps_2')}</li>
      <li>{t('onb.apps_3')}</li>
      <li>{t('onb.apps_4')}</li>
    </ol>
  {/if}

  <footer>
    <button class="link" onclick={finish}>{t('onb.skip')}</button>
    <div>
      {#if step > 0}<button class="secondary" onclick={() => step--}>{t('onb.back')}</button>{/if}
      {#if step < 2}
        <button class="primary" onclick={() => step++}>{t('onb.next')}</button>
      {:else}
        <button class="primary" onclick={finish}>{t('onb.done')}</button>
      {/if}
    </div>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 20;
  }
  .sheet {
    position: fixed;
    inset: auto 0 0 0;
    max-height: 92vh;
    overflow: auto;
    background: var(--bg);
    border-radius: 18px 18px 0 0;
    padding: 18px 16px 20px;
    z-index: 21;
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 480px;
    margin: 0 auto;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  h2 {
    margin: 0;
    font-size: 19px;
  }
  h3 {
    margin: 4px 0 0;
    font-size: 15px;
  }
  .steps {
    color: var(--sub);
    font-size: 13px;
  }
  .hint,
  small {
    color: var(--sub);
    font-size: 13px;
    margin: 0;
  }
  small {
    display: block;
    font-size: 12.5px;
  }
  .ok {
    color: var(--ok, inherit);
    margin: 0;
    font-size: 13.5px;
  }
  .warn {
    color: var(--warn);
    margin: 0;
    font-size: 13.5px;
  }
  select {
    width: 100%;
    padding: 9px 10px;
    border-radius: 10px;
    border: 1px solid var(--line);
    background: var(--card);
    color: inherit;
    font: inherit;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 16px;
    font-size: 13.5px;
  }
  ol {
    margin: 0;
    padding-left: 20px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 14px;
  }
  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 6px;
  }
  footer div {
    display: flex;
    gap: 8px;
  }
  button {
    font: inherit;
    cursor: pointer;
  }
  .primary,
  .secondary {
    border-radius: 12px;
    padding: 10px 16px;
    min-height: 40px;
    font-weight: 600;
  }
  .primary {
    border: 0;
    background: var(--accent);
    color: var(--accent-text);
  }
  .secondary {
    border: 1px solid var(--line);
    background: var(--card);
    color: inherit;
  }
  .secondary:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .link {
    border: 0;
    background: none;
    color: var(--sub);
    padding: 8px 0;
  }
  .chip {
    margin-left: 6px;
    border: 1px solid var(--line);
    background: var(--card);
    color: inherit;
    border-radius: 8px;
    padding: 2px 8px;
    font-size: 13px;
  }
</style>
