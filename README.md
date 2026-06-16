# media-maintenance

Rust CLI replacement for Lidarr maintenance workflows migrated from n8n to Dokploy schedules.

## Commands

```bash
media-maintenance album-cleanup
media-maintenance disk-cleanup
```

## CI

GitHub Actions is configured for Blacksmith runners and Blacksmith Docker build actions.

See `.env.example` and `docs/dokploy.md` for deployment configuration.
