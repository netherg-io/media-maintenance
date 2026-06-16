# Safety notes

This project is designed for unattended scheduled runs, so destructive operations must be explicit and reviewable.

## Dry-run first

Start with:

```bash
media-maintenance album-cleanup --dry-run
media-maintenance disk-cleanup --dry-run
```

Review the generated JSON reports before enabling write behaviour.

## Album cleanup writes

Album cleanup may update Lidarr artists or unmonitor duplicate releases only when all of the following are true:

- the command is not run with `--dry-run`;
- `ALBUM_APPLY=true`;
- the action is classified as safe by the duplicate matcher.

## Disk cleanup writes

Disk cleanup defaults to dry-run through `DISK_DRY_RUN=true`.

When apply mode is enabled, selected files are moved into quarantine instead of being deleted. Keep the quarantine directory on the same filesystem as the media library when possible, so moves are atomic and fast.

Recommended quarantine path:

```text
/media/.cleanup-quarantine
```

## Rollback

If a file was moved to quarantine by mistake, move it back from the run-specific quarantine directory listed in the JSON manifest.

Do not delete quarantine contents automatically until you have reviewed at least several successful runs.
