# Ev source control

Currently basically a git clone.

This is in early development and does not have a tagged release yet.
Therefore a changelog does not exist yet.

The repository format should be considered unstable until version `1.0.0`.

## Usage

### To initialize a repository:

```bash
evs init
```

### To check a repository for completeness and soundness:

```bash
evs check
```
or
```bash
evs check -vvv
```
for more details.

## TODO:

- [x] `evs init`
- [x] `evs check`
- [x] `evs cat`
- [ ] `evs add`
- [ ] `evs sub`
- [ ] `evs commit`
- [ ] `evs gc`
    > Needs good warning/user interaction
- [ ] `evs clone`
- [ ] Remote tools for evs
- [ ] Branch tools for evs
- [ ] ...
