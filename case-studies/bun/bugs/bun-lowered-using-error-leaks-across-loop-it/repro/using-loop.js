for (let i = 0; i < 2; i++) {
  try {
    using r = null;
    if (i == 0) throw new Error("boom");
  } catch (e) {
    console.log("caught", i, e.message);
  }
}
