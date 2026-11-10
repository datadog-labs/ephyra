// v is declared NMTOKEN; &#xA; is a character reference to a newline.
const xml = '<!DOCTYPE r [<!ATTLIST r v NMTOKEN #IMPLIED>]><r v="a&#xA;"/>';
console.log(JSON.stringify(Bun.XML.parse(xml)));
