# Native Discord serve live proof

Issue #4's live proof is deploy-gated: do **not** start this service or change
`pien-bridge` from a development worktree.

L1/operator steps after this PR is merged:

1. Confirm `GHQ_ROOT` resolves the oracle repository and that
   `<oracle>-oracle/ψ/memory/discord-channel-map.json` has numeric `bot_id`,
   `wind_user_id`, and mapped channel IDs.
2. Run `maw-rs discord serve <oracle> --dry-run`; it must report the intended map
   path and a non-zero mapped-channel count, without opening a gateway.
3. Under PM2, start the replacement with the required root path, for example:

   ```sh
   GHQ_ROOT="$(ghq root)" pm2 start "$(command -v maw-rs)" \
     --name pien-bridge-rs --interpreter none -- discord serve <oracle>
   ```

   It is foreground/long-lived by design; PM2 owns restart and SIGINT shutdown.
   Do not run it beside `pien-bridge` for the same bot.
4. From `wind_user_id`, post a non-bot message in a mapped channel and confirm
   the target oracle pane receives `[discord:<channel>] <message>`.
5. From that oracle, run the following and confirm the reply arrives in the
   mapped channel:

   ```sh
   maw-rs discord serve <oracle> --channel <channel> --message <reply>
   ```
6. Capture PM2 logs and the two message IDs. On a failed proof, stop only the
   replacement process and restore the existing `pien-bridge` process.
