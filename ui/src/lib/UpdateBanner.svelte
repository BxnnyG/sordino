<script lang="ts">
  import { api } from './api';
  import { t } from './i18n';
  import { store } from './store.svelte';

  let u = $derived(store.update?.available ?? null);
  let phase = $state<'idle' | 'installing' | 'done' | 'error'>('idle');
  let error = $state('');
  let showNotes = $state(false);
  let dismissed = $state('');

  // Release notes without the generic install instructions, shown as plain text.
  let notes = $derived((u?.notes ?? '').split(/\n## Install/)[0].trim());

  async function install() {
    phase = 'installing';
    error = '';
    try {
      await api.updateInstall();
      phase = 'done';
    } catch (e) {
      const msg = String(e);
      if (msg.includes('cancelled')) {
        phase = 'idle';
      } else {
        phase = 'error';
        error = msg;
      }
    }
  }
</script>

{#if u && dismissed !== u.version}
  <div class="banner update" role="status">
    <strong>{t('update.available', { v: u.version })}</strong>
    {#if phase === 'done'}
      <p>{t('update.done')}</p>
      <button class="primary small" onclick={() => api.updateRestart()}>{t('update.restart')}</button>
    {:else}
      {#if notes}
        <button class="link" onclick={() => (showNotes = !showNotes)}>{showNotes ? t('update.hide_notes') : t('update.notes')}</button>
        {#if showNotes}<pre>{notes}</pre>{/if}
      {/if}
      {#if u.can_install}
        <p class="sub">{t('update.verified')}</p>
        <div class="actions">
          <button class="primary small" disabled={phase === 'installing'} onclick={install}>
            {phase === 'installing' ? t('update.installing') : t('update.install')}
          </button>
          <button class="link" onclick={() => (dismissed = u.version)}>{t('update.later')}</button>
        </div>
      {:else}
        <p class="sub">{t(`update.reason.${u.reason ?? 'manual'}`)}</p>
        <div class="actions">
          <button class="primary small" onclick={() => api.openRepo()}>{t('update.open')}</button>
          <button class="link" onclick={() => (dismissed = u.version)}>{t('update.later')}</button>
        </div>
      {/if}
      {#if phase === 'error'}<p class="err">{t('update.failed')}: {error}</p>{/if}
    {/if}
  </div>
{/if}

<style>
  .update {
    border-radius: var(--radius);
    padding: 14px 16px;
    font-size: 14px;
    background: color-mix(in srgb, var(--accent) 10%, var(--card));
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .sub {
    margin: 0;
    color: var(--sub);
    font-size: 13px;
  }
  p {
    margin: 0;
  }
  pre {
    white-space: pre-wrap;
    font: inherit;
    font-size: 13px;
    margin: 0;
    max-height: 220px;
    overflow: auto;
    color: var(--sub);
  }
  .actions {
    display: flex;
    gap: 12px;
    align-items: center;
  }
  .primary {
    border: 0;
    border-radius: 12px;
    padding: 9px 16px;
    background: var(--accent);
    color: var(--accent-text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.7;
    cursor: default;
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    padding: 0;
    font: inherit;
    font-size: 13.5px;
    cursor: pointer;
    align-self: flex-start;
  }
  .err {
    color: var(--bad);
    font-size: 13px;
  }
</style>
