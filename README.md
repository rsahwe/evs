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

### To print a given store object:

```bash
evs cat ...
```

### To add or remove files or directories from the stage:

```bash
evs add example.txt example.dir

evs sub example.txt example.dir
```

### To commit the changes from the stage to the current branch (currently only HEAD):

```bash
evs commit -m message -n name -e email
```

### To print the commit log (default commit limit is 5):

```bash
evs log
```

### To remove unnecessary objects from the evs store:

```bash
evs gc
```

### To just print the resolved object name:

```bash
evs resolve HEAD
```

## TODO:

- [x] `evs init`
- [x] `evs check`
- [x] `evs cat`
- [x] `evs add`
- [x] `evs sub`
- [x] `evs commit`
    > `--amend`
- [x] `evs log`
- [x] `evs gc`
- [x] `evs resolve`
- [ ] `evs clone`
- [ ] `evs checkout`
- [ ] `evs status`
- [ ] `evs diff`
- [ ] Better logging (possibly tracing)
- [ ] Remote tools for evs
- [ ] Branch tools for evs
- [ ] ...
