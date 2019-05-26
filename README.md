# marks

`marks` marks line of source code and it shows with color.


# How to use

```bash
marks src/main.rs spec.txt | less -R
cat spec.txt
10
20 30
```

spec.txt's specicication is a number lists.


```bash
$ marks -h
marks 0.1.0
Taisuke Miyazaki <imishinist@gmail.com>
line marking cli tool

USAGE:
    marks --source <OPT> --spec <OPT>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

OPTIONS:
    -s, --source <OPT>    target source file
    -c, --spec <OPT>      specification fil

```
