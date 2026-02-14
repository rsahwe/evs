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

```
evs gc
```

## TODO:

- [x] `evs init`
- [x] `evs check`
- [x] `evs cat`
- [x] `evs add`
- [x] `evs sub`
- [x] `evs commit`
- [x] `evs log`
- [x] Maybe create and maintain db of store for more efficient garbage collection?
    > No, found better solution for the moment
- [x] `evs gc`
    > Needs good warning/user interaction
- [ ] `evs lookup`
- [x] Maybe change serialization format to something more efficient?
    > msgpack instead of cbor
- [ ] `evs clone`
- [ ] `evs checkout`
- [ ] `evs status`
- [ ] `evs diff`
- [ ] Remote tools for evs
- [ ] Branch tools for evs
- [ ] ...
