def use(p0: dict[str, bool]) -> None:
    if p0:
        match p0:
            case {"a": c}:
                c.bit_length()
