"""The suite's server keeps its databases out of the developer's home.

The types registry, like the other SQLite-backed gears, stores its database
under the server home. That home must be the fixture's temporary directory,
never the `~/.cf-gears` shared with the development configurations.
"""

import tempfile
from pathlib import Path


def test_registry_database_lives_in_the_temporary_home(test_env):
    home = Path(test_env.env.env["APP__SERVER__HOME_DIR"]).resolve()

    assert Path(tempfile.gettempdir()).resolve() in home.parents
    assert home != (Path.home() / ".cf-gears").resolve()
    assert (home / "types-registry" / "types_registry.db").is_file()
